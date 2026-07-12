use crate::config::{ConformancePolicy, ImportLossKind, LossPolicy};
use crate::dependency::{
    AuthorizedDependency, DependencyCache, DependencyCacheCapacityError,
    DependencyCacheDisposition, DependencyCacheEntry, DependencyCacheStorage, DependencyIdentity,
    DependencyReference, DependencyRequirement, DependencyResolver, DependencyRoleId,
    DependencySnapshot, OptionalDependencyEvidence, OptionalDependencyOutcome,
};
use crate::descriptor::TextDecodingMode;
use crate::diagnostic::{
    DiagnosticByteRange, DiagnosticCause, DiagnosticCode, DiagnosticDetailSink,
    DiagnosticLineColumn, DiagnosticPrepareCause, DiagnosticSeverity, DiagnosticSink,
    DiagnosticSource,
};
use crate::error::{
    BoundedFormatFailureDetailSink, FormatFailureEnvelope, FormatFailureSource,
    MAX_FORMAT_FAILURE_SOURCE_BYTES, ScopeAbort, ScopeAbortCause, TrustedExtensionBoundary,
};
use crate::format::discard_untrusted_panic_payload;
use crate::ledger::{
    ImportCoordinator, ImportDimension, ImportLedgerPhase, ImportProgressPhase, ImportProgressUnit,
    ImportReservation,
};
use crate::limits::ResourceLimits;
use crate::loss::{ImportLossDisposition, ImportLossEvidence};
use crate::options::{CompiledFormatOptions, FormatOptionKey};
use crate::registry::RegistryEntry;
use baozi_core::{
    Animation, AnimationId, Camera, CameraId, Light, LightId, Material, MaterialId, Mesh,
    MeshBinding, MeshId, MetadataValue, Node, NodeId, OrderedMapError, SceneBuilder,
    SceneCollection, SceneDraft, SceneSpace, SceneStorageFootprint, Skin, SkinId, Texture,
    TextureId,
};
use baozi_io::{AssetIo, AssetLength, AssetPath, AuthorityDomain, OpenedAsset, PathError};
use baozi_operation::{CapacityPlan, CapacityPlanError, CounterClass, LedgerCause};
use std::cell::RefCell;
use std::fmt;
use std::marker::PhantomData;
use std::mem::size_of;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::Arc;

pub type ParseResult<T> = Result<T, ParseFailure>;
type AssetSnapshotResult = Result<(Vec<u8>, Vec<ImportReservation>), ScopeAbortCause>;

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

/// A linear attempted-work permit. Dropping it retains attempted evidence without consumed work.
#[must_use = "complete the permit after the work is consumed"]
pub struct ImportWorkPermit<'scope, 'session> {
    scope: &'scope ParseScope<'session>,
    dimension: ImportDimension,
    amount: u64,
}

impl ImportWorkPermit<'_, '_> {
    pub fn complete(self) -> ParseResult<()> {
        self.scope.preflight_active()?;
        self.scope
            .coordinator
            .try_borrow_mut()
            .map_err(|_| self.scope.reentrant_capability_failure())?
            .checkpoint()
            .charge_consumed(self.dimension, self.amount)
            .map_err(|cause| self.scope.abort(ScopeAbortCause::Ledger(cause)))
    }
}

pub(crate) struct DiagnosticLineIndex {
    authority: baozi_io::AuthorityDomain,
    source: AssetPath,
    source_len: u64,
    line_starts: Vec<u64>,
    _reservation: ImportReservation,
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
    resolver: &'session dyn DependencyResolver,
    conformance_policy: ConformancePolicy,
    loss_policy: LossPolicy,
    limits: &'session ResourceLimits,
    coordinator: &'session RefCell<ImportCoordinator>,
    diagnostics: &'session RefCell<DiagnosticSink>,
    diagnostic_reservations: &'session RefCell<Vec<ImportReservation>>,
    diagnostic_line_indexes: &'session RefCell<Vec<DiagnosticLineIndex>>,
    dependencies: &'session RefCell<DependencyCache>,
    optional_dependency_evidence: &'session RefCell<Vec<OptionalDependencyEvidence>>,
    loss_evidence: &'session RefCell<Vec<ImportLossEvidence>>,
    draft_reservations: &'session RefCell<Vec<ImportReservation>>,
    draft_reservation_list_capacity: &'session RefCell<Option<ImportReservation>>,
    format_failure_storage: &'session RefCell<Option<ImportReservation>>,
    dependency_failure: RefCell<Option<DependencyResolutionContext>>,
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
    pub resolver: &'session dyn DependencyResolver,
    pub conformance_policy: ConformancePolicy,
    pub loss_policy: LossPolicy,
    pub limits: &'session ResourceLimits,
    pub coordinator: &'session RefCell<ImportCoordinator>,
    pub diagnostics: &'session RefCell<DiagnosticSink>,
    pub diagnostic_reservations: &'session RefCell<Vec<ImportReservation>>,
    pub diagnostic_line_indexes: &'session RefCell<Vec<DiagnosticLineIndex>>,
    pub dependencies: &'session RefCell<DependencyCache>,
    pub optional_dependency_evidence: &'session RefCell<Vec<OptionalDependencyEvidence>>,
    pub loss_evidence: &'session RefCell<Vec<ImportLossEvidence>>,
    pub draft_reservations: &'session RefCell<Vec<ImportReservation>>,
    pub draft_reservation_list_capacity: &'session RefCell<Option<ImportReservation>>,
    pub format_failure_storage: &'session RefCell<Option<ImportReservation>>,
    pub terminal: &'session RefCell<Option<ScopeAbortCause>>,
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
            conformance_policy: parts.conformance_policy,
            loss_policy: parts.loss_policy,
            limits: parts.limits,
            coordinator: parts.coordinator,
            diagnostics: parts.diagnostics,
            diagnostic_reservations: parts.diagnostic_reservations,
            diagnostic_line_indexes: parts.diagnostic_line_indexes,
            dependencies: parts.dependencies,
            optional_dependency_evidence: parts.optional_dependency_evidence,
            loss_evidence: parts.loss_evidence,
            draft_reservations: parts.draft_reservations,
            draft_reservation_list_capacity: parts.draft_reservation_list_capacity,
            format_failure_storage: parts.format_failure_storage,
            dependency_failure: RefCell::new(None),
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
        let result = {
            let mut coordinator = self
                .coordinator
                .try_borrow_mut()
                .map_err(|_| self.reentrant_capability_failure())?;
            coordinator.poll()
        };
        result.map_err(|cause| self.abort(ScopeAbortCause::Control(cause)))
    }

    pub fn charge_work(&self, amount: u64) -> ParseResult<()> {
        self.preflight_active()?;
        self.begin_work(ImportWorkClass::WorkUnits, amount)?
            .complete()
    }

    pub fn begin_work(
        &self,
        class: ImportWorkClass,
        amount: u64,
    ) -> ParseResult<ImportWorkPermit<'_, 'session>> {
        self.preflight_active()?;
        let mut coordinator = self
            .coordinator
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?;
        coordinator
            .poll()
            .map_err(|cause| self.abort(ScopeAbortCause::Control(cause)))?;
        let (dimension, item_limit) = class.accounting(self.limits);
        let mut checkpoint = coordinator.checkpoint();
        checkpoint
            .charge_attempted(dimension, amount)
            .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))?;
        drop(coordinator);

        if let Some((resource, limit)) = item_limit
            && amount > limit
        {
            return Err(self.abort(ScopeAbortCause::ResourceLimit { resource, limit }));
        }
        Ok(ImportWorkPermit {
            scope: self,
            dimension,
            amount,
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

    pub fn report_progress(
        &self,
        phase: ImportProgressPhase,
        unit: ImportProgressUnit,
        completed: u64,
        total: Option<u64>,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        self.coordinator
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .report_progress(phase, unit, completed, total)
            .map_err(|cause| self.abort(ScopeAbortCause::Control(cause)))
    }

    pub fn reserve_live(
        &self,
        storage: ImportStorageClass,
        bytes: u64,
    ) -> ParseResult<ImportLiveReservation<'_>> {
        self.preflight_active()?;
        let (phase, dimension) = storage.accounting();
        let reservation = self
            .coordinator
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .checkpoint()
            .reserve(phase, dimension, bytes)
            .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))?;
        Ok(ImportLiveReservation {
            reservation,
            scope: PhantomData,
        })
    }

    pub fn resize_live(
        &self,
        reservation: &mut ImportLiveReservation<'_>,
        bytes: u64,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        self.coordinator
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .checkpoint()
            .resize_reservation(&mut reservation.reservation, bytes)
            .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))
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
            CapacityPlanError::InvalidState { .. }
            | CapacityPlanError::StalePlan { .. }
            | CapacityPlanError::CapacityPlatformUnqualified { .. } => {
                self.abort(ScopeAbortCause::DescriptorContract {
                    code: "capacity-plan-contract-failed",
                })
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
        let charge = {
            let mut coordinator = self
                .coordinator
                .try_borrow_mut()
                .map_err(|_| self.reentrant_capability_failure())?;
            let mut checkpoint = coordinator.checkpoint();
            checkpoint
                .charge_attempted(ImportDimension::RecoveryCount, 1)
                .and_then(|()| checkpoint.charge_consumed(ImportDimension::RecoveryCount, 1))
                .and_then(|()| {
                    checkpoint.charge_attempted(ImportDimension::RecoveryRawBytes, raw_bytes)
                })
                .and_then(|()| {
                    checkpoint.charge_consumed(ImportDimension::RecoveryRawBytes, raw_bytes)
                })
                .and_then(|()| checkpoint.charge_attempted(ImportDimension::LossEvidenceCount, 1))
        };
        if let Err(cause) = charge {
            return Err(self.abort(ScopeAbortCause::Ledger(cause)));
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
        self.retain_loss_evidence(code, &source, raw_range, disposition)?;
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

    fn retain_loss_evidence(
        &self,
        code: DiagnosticCode,
        source: &DiagnosticSource<'_>,
        raw_range: DiagnosticByteRange,
        disposition: ImportLossDisposition,
    ) -> ParseResult<()> {
        let plan = {
            let evidence = self
                .loss_evidence
                .try_borrow()
                .map_err(|_| self.reentrant_capability_failure())?;
            let desired = evidence
                .len()
                .checked_add(1)
                .ok_or_else(|| self.resource_limit_exceeded("loss-evidence-capacity", u64::MAX))?;
            CapacityPlan::for_type::<ImportLossEvidence>(
                evidence.len(),
                evidence.capacity(),
                desired,
            )
            .map_err(|cause| self.capacity_plan_error("loss-evidence-capacity", cause))?
        };
        let source_envelope = u64::try_from(source.path().as_str().len())
            .map_err(|_| self.resource_limit_exceeded("loss-evidence-source", u64::MAX))?;
        let capacity_envelope = plan
            .envelope_delta_bytes()
            .map_err(|cause| self.capacity_plan_error("loss-evidence-capacity", cause))?;
        let envelope = source_envelope
            .checked_add(capacity_envelope)
            .ok_or_else(|| self.resource_limit_exceeded("loss-evidence-bytes", u64::MAX))?;
        let mut reservation = self
            .coordinator
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .checkpoint()
            .reserve(
                ImportLedgerPhase::FormatNormalization,
                ImportDimension::LossEvidenceBytes,
                envelope,
            )
            .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))?;

        let retained_source = source
            .path()
            .try_clone()
            .map_err(|_| self.allocation_failure("loss-evidence-source"))?;
        let source_bytes = u64::try_from(retained_source.capacity_bytes())
            .map_err(|_| self.resource_limit_exceeded("loss-evidence-source", u64::MAX))?;
        let allocation_envelope = source_bytes
            .checked_add(capacity_envelope)
            .ok_or_else(|| self.resource_limit_exceeded("loss-evidence-bytes", u64::MAX))?;
        if allocation_envelope != envelope {
            self.coordinator
                .try_borrow_mut()
                .map_err(|_| self.reentrant_capability_failure())?
                .checkpoint()
                .resize_reservation(&mut reservation, allocation_envelope)
                .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))?;
        }

        {
            let mut evidence = self
                .loss_evidence
                .try_borrow_mut()
                .map_err(|_| self.reentrant_capability_failure())?;
            plan.verify_preflight(evidence.len(), evidence.capacity())
                .map_err(|cause| self.capacity_plan_error("loss-evidence-capacity", cause))?;
            evidence
                .try_reserve_exact(plan.allocator_additional())
                .map_err(|_| self.allocation_failure("loss-evidence-capacity"))?;
        }
        let capacity_bytes = {
            let evidence = self
                .loss_evidence
                .try_borrow()
                .map_err(|_| self.reentrant_capability_failure())?;
            plan.finish(evidence.len(), evidence.capacity())
                .map_err(|cause| self.capacity_plan_error("loss-evidence-capacity", cause))?
        };
        let actual_bytes = source_bytes
            .checked_add(capacity_bytes)
            .ok_or_else(|| self.resource_limit_exceeded("loss-evidence-bytes", u64::MAX))?;
        self.coordinator
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .checkpoint()
            .resize_reservation(&mut reservation, actual_bytes)
            .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))?;
        let charge = {
            let mut coordinator = self
                .coordinator
                .try_borrow_mut()
                .map_err(|_| self.reentrant_capability_failure())?;
            let mut checkpoint = coordinator.checkpoint();
            checkpoint
                .charge_attempted(ImportDimension::LossEvidenceBytes, actual_bytes)
                .and_then(|()| checkpoint.charge_consumed(ImportDimension::LossEvidenceCount, 1))
                .and_then(|()| {
                    checkpoint.charge_consumed(ImportDimension::LossEvidenceBytes, actual_bytes)
                })
        };
        if let Err(cause) = charge {
            return Err(self.abort(ScopeAbortCause::Ledger(cause)));
        }

        self.loss_evidence
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .push(ImportLossEvidence::new(
                ImportLossKind::FormatRecovery,
                code,
                retained_source,
                raw_range,
                disposition,
                reservation,
            ));
        Ok(())
    }

    pub fn emit_diagnostic(
        &self,
        severity: DiagnosticSeverity,
        code: DiagnosticCode,
        message: fmt::Arguments<'_>,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        self.emit_diagnostic_event(
            severity,
            code,
            ImportProgressPhase::Decode,
            self.primary_source,
            None,
            None,
            None,
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
            severity,
            code,
            phase,
            source.path(),
            byte_range,
            line_column,
            object_path,
            details,
            message,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_diagnostic_event(
        &self,
        severity: DiagnosticSeverity,
        code: DiagnosticCode,
        phase: ImportProgressPhase,
        source: &AssetPath,
        byte_range: Option<DiagnosticByteRange>,
        line_column: Option<DiagnosticLineColumn>,
        object_path: Option<&str>,
        details: impl FnOnce(&mut dyn DiagnosticDetailSink),
        message: fmt::Arguments<'_>,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        let admission = self
            .diagnostics
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .begin(code, source, byte_range, line_column, object_path);
        let initial_failure = admission.failure().cloned();
        let initial_charge = {
            let mut coordinator = self
                .coordinator
                .try_borrow_mut()
                .map_err(|_| self.reentrant_capability_failure())?;
            let mut checkpoint = coordinator.checkpoint();
            checkpoint
                .charge_attempted(ImportDimension::DiagnosticCount, 1)
                .and_then(|()| {
                    checkpoint.charge_attempted(
                        ImportDimension::DiagnosticBytes,
                        admission.fixed_attempted_bytes(),
                    )
                })
        };
        if let Err(cause) = initial_charge {
            self.diagnostics.borrow_mut().discard_admission();
            return Err(self.abort(ScopeAbortCause::Ledger(cause)));
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
                self.coordinator
                    .try_borrow_mut()
                    .map_err(|_| self.reentrant_capability_failure())?
                    .checkpoint()
                    .reserve(
                        ImportLedgerPhase::FormatNormalization,
                        ImportDimension::DiagnosticBytes,
                        admission.payload_capacity_envelope(),
                    )
                    .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))?,
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
            self.reserve_diagnostic_reservation_slots(2)?;
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
                if let Err(cause) = self
                    .coordinator
                    .try_borrow_mut()
                    .map_err(|_| self.reentrant_capability_failure())?
                    .checkpoint()
                    .charge_attempted(
                        ImportDimension::DiagnosticBytes,
                        failure.additional_attempted_bytes,
                    )
                {
                    return Err(self.abort(ScopeAbortCause::Ledger(cause)));
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

        if let Err(cause) = self
            .coordinator
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .checkpoint()
            .charge_attempted(
                ImportDimension::DiagnosticBytes,
                prepared.additional_attempted_bytes(),
            )
        {
            self.diagnostics.borrow_mut().discard_prepared(prepared);
            return Err(self.abort(ScopeAbortCause::Ledger(cause)));
        }

        if prepared.is_stored() {
            let Some(upper_bound) = reservation.take() else {
                self.diagnostics.borrow_mut().discard_prepared(prepared);
                return Err(self.abort(ScopeAbortCause::DescriptorContract {
                    code: "stored-diagnostic-without-reservation",
                }));
            };
            let stored_bytes = prepared.stored_bytes();
            let payload_capacity_bytes = prepared.payload_capacity_bytes();
            let structural_bytes = prepared.structural_bytes();
            let mut exact = upper_bound;
            self.coordinator
                .try_borrow_mut()
                .map_err(|_| self.reentrant_capability_failure())?
                .resize_reservation(&mut exact, payload_capacity_bytes)
                .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))?;
            let Some(structural_upper_bound) = structural_reservation.take() else {
                self.diagnostics.borrow_mut().discard_prepared(prepared);
                return Err(self.abort(ScopeAbortCause::DescriptorContract {
                    code: "stored-diagnostic-without-structural-reservation",
                }));
            };
            let mut structural_exact = structural_upper_bound;
            self.coordinator
                .try_borrow_mut()
                .map_err(|_| self.reentrant_capability_failure())?
                .resize_reservation(&mut structural_exact, structural_bytes)
                .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))?;
            let consumed = {
                let mut coordinator = self
                    .coordinator
                    .try_borrow_mut()
                    .map_err(|_| self.reentrant_capability_failure())?;
                let mut checkpoint = coordinator.checkpoint();
                checkpoint
                    .charge_consumed(ImportDimension::DiagnosticCount, 1)
                    .and_then(|()| {
                        checkpoint.charge_consumed(ImportDimension::DiagnosticBytes, stored_bytes)
                    })
            };
            if let Err(cause) = consumed {
                self.diagnostics.borrow_mut().discard_prepared(prepared);
                return Err(self.abort(ScopeAbortCause::Ledger(cause)));
            }
            let mut reservations = self.diagnostic_reservations.borrow_mut();
            reservations.push(exact);
            reservations.push(structural_exact);
        }
        self.diagnostics.borrow_mut().commit(prepared);

        Ok(())
    }

    fn reserve_diagnostic_reservation_slots(&self, payload_slots: usize) -> ParseResult<()> {
        let plan = {
            let reservations = self
                .diagnostic_reservations
                .try_borrow()
                .map_err(|_| self.reentrant_capability_failure())?;
            let payload_required =
                reservations
                    .len()
                    .checked_add(payload_slots)
                    .ok_or_else(|| {
                        self.resource_limit_exceeded("diagnostic-reservation-list", u64::MAX)
                    })?;
            if payload_required <= reservations.capacity() {
                return Ok(());
            }
            let additional_slots = payload_slots.checked_add(1).ok_or_else(|| {
                self.resource_limit_exceeded("diagnostic-reservation-list", u64::MAX)
            })?;
            let desired_capacity = reservations
                .len()
                .checked_add(additional_slots)
                .ok_or_else(|| {
                    self.resource_limit_exceeded("diagnostic-reservation-list", u64::MAX)
                })?;
            CapacityPlan::for_type::<ImportReservation>(
                reservations.len(),
                reservations.capacity(),
                desired_capacity,
            )
            .map_err(|cause| self.capacity_plan_error("diagnostic-reservation-list", cause))?
        };
        let envelope = plan
            .envelope_delta_bytes()
            .map_err(|cause| self.capacity_plan_error("diagnostic-reservation-list", cause))?;
        let mut capacity_reservation =
            self.reserve_live(ImportStorageClass::FormatNormalization, envelope)?;
        {
            let mut reservations = self
                .diagnostic_reservations
                .try_borrow_mut()
                .map_err(|_| self.reentrant_capability_failure())?;
            plan.verify_preflight(reservations.len(), reservations.capacity())
                .map_err(|cause| self.capacity_plan_error("diagnostic-reservation-list", cause))?;
            reservations
                .try_reserve_exact(plan.allocator_additional())
                .map_err(|_| self.allocation_failure("diagnostic-reservation-list"))?;
        }
        let actual = {
            let reservations = self
                .diagnostic_reservations
                .try_borrow()
                .map_err(|_| self.reentrant_capability_failure())?;
            plan.finish(reservations.len(), reservations.capacity())
                .map_err(|cause| self.capacity_plan_error("diagnostic-reservation-list", cause))?
        };
        self.resize_live(&mut capacity_reservation, actual)?;
        self.diagnostic_reservations
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .push(capacity_reservation.into_reservation());
        Ok(())
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

        let work = source_len
            .checked_mul(2)
            .ok_or_else(|| self.resource_limit_exceeded("diagnostic-line-index-work", u64::MAX))?;
        let permit = self.begin_work(ImportWorkClass::DecodeWork, work)?;
        let mut line_count = 1_usize;
        for chunk in bytes.chunks(4096) {
            self.poll()?;
            line_count = chunk
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
        }
        let line_plan = CapacityPlan::for_type::<u64>(0, 0, line_count)
            .map_err(|cause| self.capacity_plan_error("diagnostic-line-index", cause))?;
        let index_plan = {
            let indexes = self
                .diagnostic_line_indexes
                .try_borrow()
                .map_err(|_| self.reentrant_capability_failure())?;
            let desired = indexes.len().checked_add(1).ok_or_else(|| {
                self.resource_limit_exceeded("diagnostic-line-index-cache", u64::MAX)
            })?;
            CapacityPlan::for_type::<DiagnosticLineIndex>(
                indexes.len(),
                indexes.capacity(),
                desired,
            )
            .map_err(|cause| self.capacity_plan_error("diagnostic-line-index-cache", cause))?
        };
        let allocation_bytes = line_plan
            .envelope_delta_bytes()
            .and_then(|bytes| {
                index_plan.envelope_delta_bytes().and_then(|index| {
                    bytes
                        .checked_add(index)
                        .ok_or(CapacityPlanError::CapacityOverflow)
                })
            })
            .and_then(|bytes| {
                bytes
                    .checked_add(source.path().as_str().len() as u64)
                    .ok_or(CapacityPlanError::CapacityOverflow)
            })
            .map_err(|cause| self.capacity_plan_error("diagnostic-line-index-bytes", cause))?;
        let mut reservation = self.reserve_live(ImportStorageClass::LineIndex, allocation_bytes)?;
        let mut line_starts = Vec::new();
        line_plan
            .verify_preflight(line_starts.len(), line_starts.capacity())
            .map_err(|cause| self.capacity_plan_error("diagnostic-line-index", cause))?;
        line_starts
            .try_reserve_exact(line_plan.allocator_additional())
            .map_err(|_| self.allocation_failure("diagnostic-line-index"))?;
        let line_capacity_bytes = line_plan
            .finish(line_starts.len(), line_starts.capacity())
            .map_err(|cause| self.capacity_plan_error("diagnostic-line-index", cause))?;
        line_starts.push(0);
        let mut offset = 0_usize;
        for chunk in bytes.chunks(4096) {
            self.poll()?;
            for (chunk_offset, byte) in chunk.iter().enumerate() {
                if *byte == b'\n' {
                    let start = offset
                        .checked_add(chunk_offset)
                        .and_then(|value| value.checked_add(1))
                        .and_then(|value| u64::try_from(value).ok())
                        .ok_or_else(|| {
                            self.resource_limit_exceeded("diagnostic-line-index-offset", u64::MAX)
                        })?;
                    line_starts.push(start);
                }
            }
            offset = offset.checked_add(chunk.len()).ok_or_else(|| {
                self.resource_limit_exceeded("diagnostic-line-index-offset", u64::MAX)
            })?;
        }
        permit.complete()?;

        let mut indexes = self
            .diagnostic_line_indexes
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?;
        index_plan
            .verify_preflight(indexes.len(), indexes.capacity())
            .map_err(|cause| self.capacity_plan_error("diagnostic-line-index-cache", cause))?;
        indexes
            .try_reserve_exact(index_plan.allocator_additional())
            .map_err(|_| self.allocation_failure("diagnostic-line-index-cache"))?;
        let index_capacity_bytes = index_plan
            .finish(indexes.len(), indexes.capacity())
            .map_err(|cause| self.capacity_plan_error("diagnostic-line-index-cache", cause))?;
        let actual_bytes = line_capacity_bytes
            .checked_add(index_capacity_bytes)
            .and_then(|bytes| bytes.checked_add(source.path().as_str().len() as u64))
            .ok_or_else(|| {
                self.resource_limit_exceeded("diagnostic-line-index-actual-bytes", u64::MAX)
            })?;
        self.resize_live(&mut reservation, actual_bytes)?;
        let source_path = source
            .path()
            .try_clone()
            .map_err(|_| self.allocation_failure("diagnostic-line-index-source"))?;
        indexes.push(DiagnosticLineIndex {
            authority: source.authority().clone(),
            source: source_path,
            source_len,
            line_starts,
            _reservation: reservation.into_reservation(),
        });
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

    pub fn required_dependency(
        &self,
        role: &DependencyRoleId,
        raw_reference: &str,
    ) -> ParseResult<DependencySnapshot<'_>> {
        self.poll_active()?;
        self.require_role(role, DependencyRequirement::Required)?;
        match self.acquire_dependency(role, raw_reference) {
            Ok(snapshot) => Ok(snapshot),
            Err(failure) => Err(self.abort_dependency_failure(failure)),
        }
    }

    pub fn optional_dependency(
        &self,
        role: &DependencyRoleId,
        raw_reference: &str,
    ) -> ParseResult<OptionalDependencyOutcome<'_>> {
        self.poll_active()?;
        self.require_role(role, DependencyRequirement::Optional)?;
        match self.acquire_dependency(role, raw_reference) {
            Ok(snapshot) => Ok(OptionalDependencyOutcome::available(snapshot)),
            Err(failure) => self.resolve_optional_failure(role, failure),
        }
    }

    pub fn scene_builder(&self) -> ParseResult<ScopedSceneBuilder<'_, 'session>> {
        self.preflight_active()?;
        self.poll()?;
        let capacity_plan = SceneBuilder::initial_capacity_plan()
            .map_err(|cause| self.abort(ScopeAbortCause::SceneBuild(cause)))?;
        let capacity_bytes = capacity_plan
            .envelope_delta_bytes()
            .map_err(|cause| self.abort(ScopeAbortCause::SceneBuild(cause)))?;
        let root_bytes = u64::try_from(size_of::<SceneBuilder>())
            .ok()
            .and_then(|bytes| bytes.checked_add(capacity_bytes))
            .ok_or_else(|| self.allocation_failure("scene-builder-root-accounting"))?;
        let root = self.reserve_live(ImportStorageClass::SceneConstruction, root_bytes)?;
        let mut scoped = ScopedSceneBuilder {
            scope: self,
            builder: SceneBuilder::try_new_with_capacity_plan(capacity_plan)
                .map_err(|cause| self.abort(ScopeAbortCause::SceneBuild(cause)))?,
            reservations: Vec::new(),
            reservation_list_capacity: None,
        };
        scoped.adopt_reservation(root)?;
        Ok(scoped)
    }

    pub fn format_failure<S>(&self, source: S) -> ParseFailure
    where
        S: FormatFailureSource,
    {
        if let Err(failure) = self.preflight_active() {
            return failure;
        }
        let failure_storage_is_active = match self.format_failure_storage.try_borrow() {
            Ok(storage) => storage.is_some(),
            Err(_) => return self.reentrant_capability_failure(),
        };
        if failure_storage_is_active {
            return self.abort(ScopeAbortCause::DescriptorContract {
                code: "format-failure-storage-already-active",
            });
        }
        let source_bytes = match u64::try_from(size_of::<S>()) {
            Ok(bytes) => bytes,
            Err(_) => {
                return self.resource_limit_exceeded(
                    "format-failure-source-storage",
                    MAX_FORMAT_FAILURE_SOURCE_BYTES,
                );
            }
        };
        if source_bytes > MAX_FORMAT_FAILURE_SOURCE_BYTES {
            return self.resource_limit_exceeded(
                "format-failure-source-storage",
                MAX_FORMAT_FAILURE_SOURCE_BYTES,
            );
        }
        let capacity_plan = match BoundedFormatFailureDetailSink::capacity_plan() {
            Ok(plan) => plan,
            Err(cause) => {
                return self.capacity_plan_error("format-failure-detail-storage", cause);
            }
        };
        let detail_envelope_bytes = match capacity_plan.envelope_bytes() {
            Ok(bytes) => bytes,
            Err(cause) => {
                return self.capacity_plan_error("format-failure-detail-storage", cause);
            }
        };
        let envelope_bytes = match detail_envelope_bytes.checked_add(source_bytes) {
            Some(bytes) => bytes,
            None => {
                return self.resource_limit_exceeded("format-failure-storage", u64::MAX);
            }
        };
        let mut storage =
            match self.reserve_live(ImportStorageClass::FormatNormalization, envelope_bytes) {
                Ok(reservation) => reservation,
                Err(failure) => return failure,
            };
        let (mut details, actual_capacity_bytes) =
            match BoundedFormatFailureDetailSink::try_allocate(capacity_plan) {
                Ok(allocated) => allocated,
                Err(cause) => {
                    return self.abort(ScopeAbortCause::FormatFailureDetail(cause));
                }
            };
        let actual_storage_bytes = match actual_capacity_bytes.checked_add(source_bytes) {
            Some(bytes) => bytes,
            None => {
                return self.resource_limit_exceeded("format-failure-storage", u64::MAX);
            }
        };
        if let Err(failure) = self.resize_live(&mut storage, actual_storage_bytes) {
            return failure;
        }
        {
            let mut failure_storage = match self.format_failure_storage.try_borrow_mut() {
                Ok(storage) => storage,
                Err(_) => return self.reentrant_capability_failure(),
            };
            if failure_storage.is_some() {
                return self.abort(ScopeAbortCause::DescriptorContract {
                    code: "format-failure-storage-already-active",
                });
            }
            *failure_storage = Some(storage.into_reservation());
        }
        let code = match catch_unwind(AssertUnwindSafe(|| source.code())) {
            Ok(code) => code,
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                return self.abort(ScopeAbortCause::TrustedExtension {
                    boundary: TrustedExtensionBoundary::FormatFailureDetail,
                    code: "format-failure-code-panicked",
                });
            }
        };
        if let Err(failure) = self.preflight_active() {
            return failure;
        }
        if DiagnosticCode::new(code).is_err() {
            return self.abort(ScopeAbortCause::DescriptorContract {
                code: "invalid-format-failure-code",
            });
        }
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| source.visit_details(&mut details)))
        {
            discard_untrusted_panic_payload(payload);
            return self.abort(ScopeAbortCause::TrustedExtension {
                boundary: TrustedExtensionBoundary::FormatFailureDetail,
                code: "format-failure-detail-panicked",
            });
        }
        if let Err(failure) = self.preflight_active() {
            return failure;
        }
        let details = match details.finish() {
            Ok(details) => details,
            Err(cause) => return self.abort(ScopeAbortCause::FormatFailureDetail(cause)),
        };
        let envelope =
            FormatFailureEnvelope::seal(self.entry.descriptor_handle(), code, details, source);
        ParseFailure::format(envelope)
    }

    pub(crate) fn take_terminal_abort(&self) -> Option<ScopeAbort> {
        self.terminal.borrow_mut().take().map(ScopeAbort::new)
    }

    fn require_role(
        &self,
        role: &DependencyRoleId,
        expected: DependencyRequirement,
    ) -> ParseResult<()> {
        match self.entry.descriptor().dependency_requirement(role) {
            Some(actual) if actual == expected => Ok(()),
            Some(_) => Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "dependency-requiredness-mismatch",
            })),
            None => Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "dependency-role-not-declared",
            })),
        }
    }

    fn dependency_failure(
        &self,
        identity: Option<DependencyIdentity>,
        normalized_path: MeteredDependencyPath,
        cause: ScopeAbortCause,
    ) -> DependencyResolutionFailure {
        self.store_dependency_failure(DependencyResolutionContext {
            identity,
            normalized_path: Some(normalized_path),
            cause: DependencyResolutionCause::Owned(cause),
            cache_disposition: DependencyCacheDisposition::Fresh,
        })
    }

    fn cached_dependency_failure(
        &self,
        identity: DependencyIdentity,
        normalized_path: MeteredDependencyPath,
        index: usize,
        cache_disposition: DependencyCacheDisposition,
    ) -> DependencyResolutionFailure {
        self.store_dependency_failure(DependencyResolutionContext {
            identity: Some(identity),
            normalized_path: Some(normalized_path),
            cause: DependencyResolutionCause::Cached { index },
            cache_disposition,
        })
    }

    fn dependency_failure_without_path(
        &self,
        cause: ScopeAbortCause,
    ) -> DependencyResolutionFailure {
        self.store_dependency_failure(DependencyResolutionContext {
            identity: None,
            normalized_path: None,
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
                identity: None,
                normalized_path: None,
                cause: DependencyResolutionCause::Owned(ScopeAbortCause::DescriptorContract {
                    code: "dependency-failure-marker-without-context",
                }),
                cache_disposition: DependencyCacheDisposition::Fresh,
            })
    }

    fn normalize_dependency_path(
        &self,
        raw_reference: &str,
    ) -> Result<MeteredDependencyPath, ScopeAbortCause> {
        let envelope =
            dependency_normalization_capacity_envelope(self.primary_source, raw_reference)?;
        let reservation = self.reserve_dependency_storage_cause(envelope)?;
        let path = self
            .primary_source
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
        mut reservation: ImportReservation,
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

    fn acquire_dependency(
        &self,
        role: &DependencyRoleId,
        raw_reference: &str,
    ) -> Result<DependencySnapshot<'_>, DependencyResolutionFailure> {
        self.charge_logical_reference()?;
        if raw_reference.len() > self.limits.max_path_length {
            let primary_source = self
                .clone_metered_dependency_path(self.primary_source, "dependency-failure-source")
                .map_err(|cause| self.dependency_failure_without_path(cause))?;
            return Err(self.dependency_failure(
                None,
                primary_source,
                ScopeAbortCause::ResourceLimit {
                    resource: "dependency-reference-bytes",
                    limit: self.limits.max_path_length as u64,
                },
            ));
        }
        let normalized = match self.normalize_dependency_path(raw_reference) {
            Ok(normalized) => normalized,
            Err(cause @ ScopeAbortCause::Path(_)) => {
                let primary_source = self
                    .clone_metered_dependency_path(self.primary_source, "dependency-failure-source")
                    .map_err(|clone_cause| self.dependency_failure_without_path(clone_cause))?;
                return Err(self.dependency_failure(None, primary_source, cause));
            }
            Err(cause) => return Err(self.dependency_failure_without_path(cause)),
        };
        let reference = DependencyReference::new(raw_reference, normalized.path());
        let authorized = match catch_unwind(AssertUnwindSafe(|| {
            self.resolver.authorize(
                self.primary_io,
                self.primary_authority,
                self.primary_source,
                role,
                reference,
            )
        })) {
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                return Err(self.dependency_failure(
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
                    None,
                    normalized,
                    ScopeAbortCause::DependencyPolicy(cause),
                ));
            }
            Ok(Ok(authorized)) => authorized,
        };

        self.acquire_authorized(normalized, authorized)
    }

    fn acquire_authorized(
        &self,
        normalized: MeteredDependencyPath,
        authorized: AuthorizedDependency<'_>,
    ) -> Result<DependencySnapshot<'_>, DependencyResolutionFailure> {
        let (io, identity) = authorized.into_parts();
        let backend_authority = if std::ptr::eq(io, self.primary_io) {
            self.primary_authority.clone()
        } else {
            match catch_unwind(AssertUnwindSafe(|| io.authority_domain())) {
                Ok(authority) => authority,
                Err(payload) => {
                    discard_untrusted_panic_payload(payload);
                    return Err(self.dependency_failure(
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
                Some(identity),
                normalized,
                ScopeAbortCause::TrustedExtension {
                    boundary: TrustedExtensionBoundary::AssetIo,
                    code: "authorized-backend-authority-mismatch",
                },
            ));
        }
        let cached = {
            let cache = self.dependencies.borrow();
            match cache.get(&identity) {
                Some((_, DependencyCacheEntry::Available(snapshot))) => {
                    Some(Arc::clone(&snapshot.bytes))
                }
                Some((index, DependencyCacheEntry::Failed(_))) => {
                    return Err(self.cached_dependency_failure(
                        identity,
                        normalized,
                        index,
                        DependencyCacheDisposition::Reused,
                    ));
                }
                Some((_, DependencyCacheEntry::Resolving)) => {
                    return Err(self.dependency_failure(
                        Some(identity),
                        normalized,
                        ScopeAbortCause::DescriptorContract {
                            code: "dependency-resolution-reentered",
                        },
                    ));
                }
                Some((_, DependencyCacheEntry::FailureConsumed)) => {
                    return Err(self.dependency_failure(
                        Some(identity),
                        normalized,
                        ScopeAbortCause::DescriptorContract {
                            code: "dependency-cache-failure-was-consumed",
                        },
                    ));
                }
                None => None,
            }
        };
        if let Some(bytes) = cached {
            let (snapshot_identity, identity_path_reservation) =
                match self.clone_dependency_identity(&identity) {
                    Ok(identity) => identity,
                    Err(cause) => {
                        return Err(self.dependency_failure(Some(identity), normalized, cause));
                    }
                };
            return Ok(DependencySnapshot::new(
                snapshot_identity,
                bytes,
                DependencyCacheDisposition::Reused,
                identity_path_reservation,
            ));
        }

        if let Err(cause) = self.reserve_dependency_cache_capacity() {
            return Err(self.dependency_failure(Some(identity), normalized, cause));
        }
        let cache_identity_path_reservation =
            match self.reserve_dependency_storage_cause(identity.path().as_str().len() as u64) {
                Ok(reservation) => reservation,
                Err(cause) => {
                    return Err(self.dependency_failure(Some(identity), normalized, cause));
                }
            };
        let cache_identity = match identity.try_clone() {
            Ok(cache_identity) => cache_identity,
            Err(_) => {
                return Err(self.dependency_failure(
                    Some(identity),
                    normalized,
                    ScopeAbortCause::Allocation {
                        resource: "dependency-cache-identity",
                    },
                ));
            }
        };
        let marked = match self
            .dependencies
            .borrow_mut()
            .mark_resolving(cache_identity, cache_identity_path_reservation)
        {
            Ok(marked) => marked,
            Err(()) => {
                return Err(self.dependency_failure(
                    Some(identity),
                    normalized,
                    ScopeAbortCause::Allocation {
                        resource: "dependency-cache-entry",
                    },
                ));
            }
        };
        if !marked {
            return Err(self.dependency_failure(
                Some(identity),
                normalized,
                ScopeAbortCause::DescriptorContract {
                    code: "dependency-cache-state-raced",
                },
            ));
        }

        let acquired =
            self.open_and_read_dependency(io, &identity)
                .and_then(|(bytes, reservations)| {
                    let (snapshot_bytes, sharing_reservation) =
                        self.share_dependency_snapshot(bytes)?;
                    let (snapshot_identity, identity_path_reservation) =
                        self.clone_dependency_identity(&identity)?;
                    Ok((
                        snapshot_bytes,
                        reservations,
                        sharing_reservation,
                        snapshot_identity,
                        identity_path_reservation,
                    ))
                });
        match acquired {
            Ok((
                snapshot_bytes,
                reservations,
                sharing_reservation,
                snapshot_identity,
                identity_path_reservation,
            )) => {
                if !self.dependencies.borrow_mut().store_available(
                    &identity,
                    Arc::clone(&snapshot_bytes),
                    reservations,
                    sharing_reservation,
                ) {
                    self.dependencies.borrow_mut().remove(&identity);
                    return Err(self.dependency_failure(
                        Some(identity),
                        normalized,
                        ScopeAbortCause::DescriptorContract {
                            code: "dependency-cache-transition-missing",
                        },
                    ));
                }
                Ok(DependencySnapshot::new(
                    snapshot_identity,
                    snapshot_bytes,
                    DependencyCacheDisposition::Fresh,
                    identity_path_reservation,
                ))
            }
            Err(cause) => {
                Err(self.finish_dependency_resolution_failure(identity, normalized, cause))
            }
        }
    }

    fn finish_dependency_resolution_failure(
        &self,
        identity: DependencyIdentity,
        normalized: MeteredDependencyPath,
        cause: ScopeAbortCause,
    ) -> DependencyResolutionFailure {
        if !cacheable_dependency_cause(&cause) {
            self.dependencies.borrow_mut().remove(&identity);
            return self.dependency_failure(Some(identity), normalized, cause);
        }

        let cached_index = self
            .dependencies
            .borrow_mut()
            .store_failed(&identity, cause);
        match cached_index {
            Ok(index) => self.cached_dependency_failure(
                identity,
                normalized,
                index,
                DependencyCacheDisposition::Fresh,
            ),
            Err(_) => {
                self.dependencies.borrow_mut().remove(&identity);
                self.dependency_failure(
                    Some(identity),
                    normalized,
                    ScopeAbortCause::DescriptorContract {
                        code: "dependency-cache-transition-missing",
                    },
                )
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
        read_opened_asset(
            self.coordinator,
            opened,
            ImportLedgerPhase::DependencyAcquisition,
            ImportProgressPhase::AcquireDependency,
            ImportDimension::DependencyBytes,
            self.limits.max_dependency_asset_bytes,
            self.limits.max_total_asset_bytes,
        )
    }

    fn share_dependency_snapshot(
        &self,
        bytes: Vec<u8>,
    ) -> Result<(Arc<Vec<u8>>, ImportReservation), ScopeAbortCause> {
        const CONTROL_WORDS: usize = 4;
        let control_bytes = size_of::<Vec<u8>>()
            .checked_add(size_of::<usize>().checked_mul(CONTROL_WORDS).ok_or(
                ScopeAbortCause::ResourceLimit {
                    resource: "dependency-snapshot-sharing",
                    limit: u64::MAX,
                },
            )?)
            .ok_or(ScopeAbortCause::ResourceLimit {
                resource: "dependency-snapshot-sharing",
                limit: u64::MAX,
            })?;
        let reservation =
            self.reserve_dependency_storage_cause(u64::try_from(control_bytes).map_err(|_| {
                ScopeAbortCause::ResourceLimit {
                    resource: "dependency-snapshot-sharing",
                    limit: u64::MAX,
                }
            })?)?;
        Ok((Arc::new(bytes), reservation))
    }

    fn clone_dependency_identity(
        &self,
        source: &DependencyIdentity,
    ) -> Result<(DependencyIdentity, ImportReservation), ScopeAbortCause> {
        let reservation =
            self.reserve_dependency_storage_cause(source.path().as_str().len() as u64)?;
        let identity = source
            .try_clone()
            .map_err(|_| ScopeAbortCause::Allocation {
                resource: "dependency-snapshot-identity",
            })?;
        Ok((identity, reservation))
    }

    fn reserve_dependency_cache_capacity(&self) -> Result<(), ScopeAbortCause> {
        let (plan, entry_envelope, index_envelope, mut entry_reservation, mut index_reservation) = {
            let mut cache = self.dependencies.try_borrow_mut().map_err(|_| {
                ScopeAbortCause::DescriptorContract {
                    code: "parse-capability-reentered",
                }
            })?;
            let plan = cache
                .capacity_plan()
                .map_err(dependency_cache_capacity_cause)?;
            if !plan.requires_apply() {
                return Ok(());
            }
            let entry_envelope = plan
                .temporary_envelope_bytes(DependencyCacheStorage::Entries)
                .map_err(dependency_cache_capacity_cause)?;
            let index_envelope = plan
                .temporary_envelope_bytes(DependencyCacheStorage::Index)
                .map_err(dependency_cache_capacity_cause)?;
            let entry_reservation =
                cache.take_capacity_reservation(DependencyCacheStorage::Entries);
            let index_reservation = cache.take_capacity_reservation(DependencyCacheStorage::Index);
            (
                plan,
                entry_envelope,
                index_envelope,
                entry_reservation,
                index_reservation,
            )
        };

        let entry_prior_bytes = entry_reservation
            .as_ref()
            .map_or(0, ImportReservation::bytes);
        let index_prior_bytes = index_reservation
            .as_ref()
            .map_or(0, ImportReservation::bytes);
        // Replacements overlap the live containers, so each storage class admits
        // its temporary allocation in addition to its retained reservation.
        if let Err(cause) = self.grow_dependency_cache_reservation(
            DependencyCacheStorage::Entries,
            entry_envelope,
            &mut entry_reservation,
        ) {
            self.restore_dependency_cache_reservations(
                entry_reservation,
                entry_prior_bytes,
                index_reservation,
                index_prior_bytes,
            );
            return Err(cause);
        }
        if let Err(cause) = self.grow_dependency_cache_reservation(
            DependencyCacheStorage::Index,
            index_envelope,
            &mut index_reservation,
        ) {
            self.restore_dependency_cache_reservations(
                entry_reservation,
                entry_prior_bytes,
                index_reservation,
                index_prior_bytes,
            );
            return Err(cause);
        }

        let actual = match self.dependencies.try_borrow_mut() {
            Ok(mut cache) => cache
                .apply_capacity_plan(plan)
                .map_err(dependency_cache_capacity_cause),
            Err(_) => Err(ScopeAbortCause::DescriptorContract {
                code: "parse-capability-reentered",
            }),
        };

        let actual = match actual {
            Ok(actual) => actual,
            Err(cause) => {
                self.restore_dependency_cache_reservations(
                    entry_reservation,
                    entry_prior_bytes,
                    index_reservation,
                    index_prior_bytes,
                );
                return Err(cause);
            }
        };

        let mut resize_failure = None;
        for (storage, reservation) in [
            (DependencyCacheStorage::Entries, &mut entry_reservation),
            (DependencyCacheStorage::Index, &mut index_reservation),
        ] {
            let Some(retained_bytes) = actual.retained_bytes(storage) else {
                continue;
            };
            let Some(reservation) = reservation.as_mut() else {
                resize_failure = Some(ScopeAbortCause::DescriptorContract {
                    code: "dependency-cache-capacity-reservation-missing",
                });
                continue;
            };
            let resize = match self.coordinator.try_borrow_mut() {
                Ok(mut coordinator) => coordinator
                    .checkpoint()
                    .resize_reservation(reservation, retained_bytes)
                    .map_err(ScopeAbortCause::Ledger),
                Err(_) => Err(ScopeAbortCause::DescriptorContract {
                    code: "parse-capability-reentered",
                }),
            };
            if let Err(cause) = resize {
                resize_failure.get_or_insert(cause);
            }
        }
        let mut cache = self.dependencies.borrow_mut();
        cache.retain_capacity_reservation(DependencyCacheStorage::Entries, entry_reservation);
        cache.retain_capacity_reservation(DependencyCacheStorage::Index, index_reservation);
        drop(cache);
        if let Some(cause) = resize_failure {
            return Err(cause);
        }
        Ok(())
    }

    fn grow_dependency_cache_reservation(
        &self,
        storage: DependencyCacheStorage,
        temporary_envelope: u64,
        reservation: &mut Option<ImportReservation>,
    ) -> Result<(), ScopeAbortCause> {
        if temporary_envelope == 0 {
            return Ok(());
        }
        let prior_bytes = reservation.as_ref().map_or(0, ImportReservation::bytes);
        let target = prior_bytes.checked_add(temporary_envelope).ok_or_else(|| {
            dependency_cache_capacity_cause(DependencyCacheCapacityError::Capacity {
                storage,
                cause: CapacityPlanError::CapacityOverflow,
            })
        })?;
        if let Some(reservation) = reservation.as_mut() {
            self.coordinator
                .try_borrow_mut()
                .map_err(|_| ScopeAbortCause::DescriptorContract {
                    code: "parse-capability-reentered",
                })?
                .checkpoint()
                .resize_reservation(reservation, target)
                .map_err(ScopeAbortCause::Ledger)
        } else {
            *reservation = Some(
                self.coordinator
                    .try_borrow_mut()
                    .map_err(|_| ScopeAbortCause::DescriptorContract {
                        code: "parse-capability-reentered",
                    })?
                    .checkpoint()
                    .reserve(
                        ImportLedgerPhase::DependencyAcquisition,
                        ImportDimension::ScratchBytes,
                        target,
                    )
                    .map_err(ScopeAbortCause::Ledger)?,
            );
            Ok(())
        }
    }

    fn restore_dependency_cache_reservations(
        &self,
        mut entry_reservation: Option<ImportReservation>,
        entry_prior_bytes: u64,
        mut index_reservation: Option<ImportReservation>,
        index_prior_bytes: u64,
    ) {
        for (reservation, prior_bytes) in [
            (&mut entry_reservation, entry_prior_bytes),
            (&mut index_reservation, index_prior_bytes),
        ] {
            if prior_bytes == 0 {
                drop(reservation.take());
            } else if let Some(reservation) = reservation.as_mut()
                && let Ok(mut coordinator) = self.coordinator.try_borrow_mut()
            {
                let _ = coordinator
                    .checkpoint()
                    .resize_reservation(reservation, prior_bytes);
            }
        }
        let mut cache = self.dependencies.borrow_mut();
        cache.retain_capacity_reservation(DependencyCacheStorage::Entries, entry_reservation);
        cache.retain_capacity_reservation(DependencyCacheStorage::Index, index_reservation);
    }

    fn charge_logical_reference(&self) -> Result<(), DependencyResolutionFailure> {
        let result = {
            let mut coordinator = self.coordinator.borrow_mut();
            let mut checkpoint = coordinator.checkpoint();
            checkpoint
                .charge_attempted(ImportDimension::DependencyReference, 1)
                .and_then(|()| checkpoint.charge_consumed(ImportDimension::DependencyReference, 1))
        };
        let Err(cause) = result else {
            return Ok(());
        };
        let source = self
            .clone_metered_dependency_path(self.primary_source, "dependency-failure-source")
            .map_err(|clone_cause| self.dependency_failure_without_path(clone_cause))?;
        Err(self.dependency_failure(None, source, ScopeAbortCause::Ledger(cause)))
    }

    fn charge_open_attempt(&self) -> Result<(), ScopeAbortCause> {
        self.coordinator
            .borrow_mut()
            .checkpoint()
            .charge_attempted(ImportDimension::AssetOpen, 1)
            .map_err(ScopeAbortCause::Ledger)
    }

    fn charge_open_consumed(&self) -> Result<(), ScopeAbortCause> {
        self.coordinator
            .borrow_mut()
            .checkpoint()
            .charge_consumed(ImportDimension::AssetOpen, 1)
            .map_err(ScopeAbortCause::Ledger)
    }

    fn poll_cause(&self) -> Result<(), ScopeAbortCause> {
        self.coordinator
            .borrow_mut()
            .poll()
            .map_err(ScopeAbortCause::Control)
    }

    fn resolve_optional_failure(
        &self,
        role: &DependencyRoleId,
        failure: DependencyResolutionFailure,
    ) -> ParseResult<OptionalDependencyOutcome<'_>> {
        let failure = self.take_dependency_failure(failure);
        let (eligible, owner) = {
            let cache = self
                .dependencies
                .try_borrow()
                .map_err(|_| self.reentrant_capability_failure())?;
            let cause = failure.cause(&cache).map_err(|cause| self.abort(cause))?;
            (optional_eligible(cause), cause.owner())
        };
        if !self
            .loss_policy
            .allows(ImportLossKind::OptionalDependencyUnavailable)
            || !eligible
        {
            return Err(self.abort_dependency_context(failure));
        }
        let Some(normalized_path) = failure.normalized_path.as_ref() else {
            return Err(self.abort_dependency_context(failure));
        };
        let evidence = self.clone_optional_dependency_evidence(
            failure.identity.as_ref(),
            normalized_path.path(),
            &owner,
            failure.cache_disposition,
        )?;
        let mut retained_evidence = self.clone_optional_dependency_evidence(
            failure.identity.as_ref(),
            normalized_path.path(),
            &owner,
            failure.cache_disposition,
        )?;
        if let Some(reservation) = self.reserve_optional_evidence_capacity()? {
            retained_evidence.retain_container_capacity(reservation);
        }
        {
            let mut retained = self
                .optional_dependency_evidence
                .try_borrow_mut()
                .map_err(|_| self.reentrant_capability_failure())?;
            retained.push(retained_evidence);
        }
        self.emit_diagnostic(
            DiagnosticSeverity::Warning,
            DiagnosticCode::new("import.optional_dependency_unavailable")
                .map_err(|cause| self.abort(ScopeAbortCause::Diagnostic(cause)))?,
            format_args!("optional dependency role {role} is unavailable"),
        )?;
        Ok(OptionalDependencyOutcome::unavailable(evidence))
    }

    fn abort_dependency_failure(&self, failure: DependencyResolutionFailure) -> ParseFailure {
        self.abort_dependency_context(self.take_dependency_failure(failure))
    }

    fn abort_dependency_context(&self, failure: DependencyResolutionContext) -> ParseFailure {
        let (cause, normalized_path) = match self.dependencies.try_borrow_mut() {
            Ok(mut cache) => failure.into_cause(&mut cache),
            Err(_) => (
                ScopeAbortCause::DescriptorContract {
                    code: "parse-capability-reentered",
                },
                failure.normalized_path,
            ),
        };
        let parse_failure = self.abort(cause);
        drop(normalized_path);
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

    fn reserve_optional_evidence_capacity(&self) -> ParseResult<Option<ImportReservation>> {
        let plan = {
            let retained = self
                .optional_dependency_evidence
                .try_borrow()
                .map_err(|_| self.reentrant_capability_failure())?;
            let desired_capacity = retained.len().checked_add(1).ok_or_else(|| {
                self.resource_limit_exceeded("optional-dependency-evidence-capacity", u64::MAX)
            })?;
            CapacityPlan::for_type::<OptionalDependencyEvidence>(
                retained.len(),
                retained.capacity(),
                desired_capacity,
            )
            .map_err(|cause| {
                self.capacity_plan_error("optional-dependency-evidence-capacity", cause)
            })?
        };
        let envelope = plan.envelope_delta_bytes().map_err(|cause| {
            self.capacity_plan_error("optional-dependency-evidence-capacity", cause)
        })?;
        if envelope == 0 {
            return Ok(None);
        }
        let mut reservation = self.reserve_dependency_storage(envelope)?;
        {
            let mut retained = self
                .optional_dependency_evidence
                .try_borrow_mut()
                .map_err(|_| self.reentrant_capability_failure())?;
            plan.verify_preflight(retained.len(), retained.capacity())
                .map_err(|cause| {
                    self.capacity_plan_error("optional-dependency-evidence-capacity", cause)
                })?;
            retained
                .try_reserve_exact(plan.allocator_additional())
                .map_err(|_| {
                    self.abort(ScopeAbortCause::Allocation {
                        resource: "optional-dependency-evidence-capacity",
                    })
                })?;
        }
        let actual = {
            let retained = self
                .optional_dependency_evidence
                .try_borrow()
                .map_err(|_| self.reentrant_capability_failure())?;
            plan.finish(retained.len(), retained.capacity())
                .map_err(|cause| {
                    self.capacity_plan_error("optional-dependency-evidence-capacity", cause)
                })?
        };
        self.coordinator
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .checkpoint()
            .resize_reservation(&mut reservation, actual)
            .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))?;
        Ok(Some(reservation))
    }

    fn reserve_dependency_storage(&self, bytes: u64) -> ParseResult<ImportReservation> {
        self.reserve_dependency_storage_cause(bytes)
            .map_err(|cause| self.abort(cause))
    }

    fn reserve_dependency_storage_cause(
        &self,
        bytes: u64,
    ) -> Result<ImportReservation, ScopeAbortCause> {
        self.coordinator
            .try_borrow_mut()
            .map_err(|_| ScopeAbortCause::DescriptorContract {
                code: "parse-capability-reentered",
            })?
            .checkpoint()
            .reserve(
                ImportLedgerPhase::DependencyAcquisition,
                ImportDimension::ScratchBytes,
                bytes,
            )
            .map_err(ScopeAbortCause::Ledger)
    }

    fn resize_dependency_storage_cause(
        &self,
        reservation: &mut ImportReservation,
        bytes: u64,
    ) -> Result<(), ScopeAbortCause> {
        self.coordinator
            .try_borrow_mut()
            .map_err(|_| ScopeAbortCause::DescriptorContract {
                code: "parse-capability-reentered",
            })?
            .checkpoint()
            .resize_reservation(reservation, bytes)
            .map_err(ScopeAbortCause::Ledger)
    }

    fn abort(&self, cause: ScopeAbortCause) -> ParseFailure {
        let mut terminal = self.terminal.borrow_mut();
        if terminal.is_none() {
            *terminal = Some(cause);
        }
        ParseFailure::scope()
    }

    fn preflight_active(&self) -> ParseResult<()> {
        if self.terminal.borrow().is_some() {
            Err(ParseFailure::scope())
        } else {
            Ok(())
        }
    }

    fn reentrant_capability_failure(&self) -> ParseFailure {
        self.abort(ScopeAbortCause::DescriptorContract {
            code: "parse-capability-reentered",
        })
    }

    fn seal_draft_reservations(
        &self,
        reservations: &mut Vec<ImportReservation>,
        required_bytes: u64,
    ) -> ParseResult<()> {
        let current_bytes = reservations.iter().try_fold(0_u64, |total, reservation| {
            total.checked_add(reservation.bytes()).ok_or_else(|| {
                self.abort(ScopeAbortCause::DescriptorContract {
                    code: "draft-reservation-bytes-overflow",
                })
            })
        })?;
        if current_bytes < required_bytes {
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "draft-storage-was-not-reserved-before-allocation",
            }));
        }

        self.reserve_draft_reservation_slots(reservations.len())?;
        let mut session_reservations = self
            .draft_reservations
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?;
        session_reservations.append(reservations);
        Ok(())
    }

    fn reserve_draft_reservation_slots(&self, payload_slots: usize) -> ParseResult<()> {
        if payload_slots == 0 {
            return Ok(());
        }
        let plan = {
            let reservations = self
                .draft_reservations
                .try_borrow()
                .map_err(|_| self.reentrant_capability_failure())?;
            let desired_capacity =
                reservations
                    .len()
                    .checked_add(payload_slots)
                    .ok_or_else(|| {
                        self.resource_limit_exceeded("draft-reservation-side-channel", u64::MAX)
                    })?;
            CapacityPlan::for_type::<ImportReservation>(
                reservations.len(),
                reservations.capacity(),
                desired_capacity,
            )
            .map_err(|cause| self.capacity_plan_error("draft-reservation-side-channel", cause))?
        };
        let envelope_delta = plan
            .envelope_delta_bytes()
            .map_err(|cause| self.capacity_plan_error("draft-reservation-side-channel", cause))?;
        if envelope_delta == 0 {
            return Ok(());
        }
        let prior_reservation = self
            .draft_reservation_list_capacity
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .take();
        let prior_bytes = prior_reservation
            .as_ref()
            .map_or(0, ImportReservation::bytes);
        let envelope = prior_bytes.checked_add(envelope_delta).ok_or_else(|| {
            self.resource_limit_exceeded("draft-reservation-side-channel", u64::MAX)
        })?;
        let mut reservation = match prior_reservation {
            Some(mut reservation) => {
                self.coordinator
                    .try_borrow_mut()
                    .map_err(|_| self.reentrant_capability_failure())?
                    .checkpoint()
                    .resize_reservation(&mut reservation, envelope)
                    .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))?;
                reservation
            }
            None => self
                .coordinator
                .try_borrow_mut()
                .map_err(|_| self.reentrant_capability_failure())?
                .checkpoint()
                .reserve(
                    ImportLedgerPhase::SceneConstruction,
                    ImportDimension::ScratchBytes,
                    envelope,
                )
                .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))?,
        };
        let actual_delta = {
            let mut reservations = self
                .draft_reservations
                .try_borrow_mut()
                .map_err(|_| self.reentrant_capability_failure())?;
            plan.verify_preflight(reservations.len(), reservations.capacity())
                .map_err(|cause| {
                    self.capacity_plan_error("draft-reservation-side-channel", cause)
                })?;
            reservations
                .try_reserve_exact(plan.allocator_additional())
                .map_err(|_| self.allocation_failure("draft-reservation-side-channel"))?;
            plan.finish(reservations.len(), reservations.capacity())
                .map_err(|cause| {
                    self.capacity_plan_error("draft-reservation-side-channel", cause)
                })?
        };
        let actual = prior_bytes.checked_add(actual_delta).ok_or_else(|| {
            self.resource_limit_exceeded("draft-reservation-side-channel", u64::MAX)
        })?;
        self.coordinator
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .checkpoint()
            .resize_reservation(&mut reservation, actual)
            .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))?;
        *self
            .draft_reservation_list_capacity
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())? = Some(reservation);
        Ok(())
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
    const fn accounting(self) -> (ImportLedgerPhase, ImportDimension) {
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

pub struct ImportLiveReservation<'scope> {
    reservation: ImportReservation,
    scope: PhantomData<&'scope ()>,
}

impl ImportLiveReservation<'_> {
    pub fn bytes(&self) -> u64 {
        self.reservation.bytes()
    }

    fn into_reservation(self) -> ImportReservation {
        self.reservation
    }
}

struct DependencyResolutionFailure;

struct MeteredDependencyPath {
    path: AssetPath,
    _reservation: ImportReservation,
}

impl MeteredDependencyPath {
    fn new(path: AssetPath, reservation: ImportReservation) -> Self {
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
    identity: Option<DependencyIdentity>,
    normalized_path: Option<MeteredDependencyPath>,
    cause: DependencyResolutionCause,
    cache_disposition: DependencyCacheDisposition,
}

enum DependencyResolutionCause {
    Owned(ScopeAbortCause),
    Cached { index: usize },
}

impl DependencyResolutionContext {
    fn cause<'a>(
        &'a self,
        cache: &'a DependencyCache,
    ) -> Result<&'a ScopeAbortCause, ScopeAbortCause> {
        match &self.cause {
            DependencyResolutionCause::Owned(cause) => Ok(cause),
            DependencyResolutionCause::Cached { index } => {
                cache
                    .failed(*index)
                    .ok_or(ScopeAbortCause::DescriptorContract {
                        code: "dependency-cache-failure-index-invalid",
                    })
            }
        }
    }

    fn into_cause(
        self,
        cache: &mut DependencyCache,
    ) -> (ScopeAbortCause, Option<MeteredDependencyPath>) {
        let Self {
            normalized_path,
            cause,
            ..
        } = self;
        let cause = match cause {
            DependencyResolutionCause::Owned(cause) => cause,
            DependencyResolutionCause::Cached { index } => {
                cache
                    .take_failed(index)
                    .unwrap_or(ScopeAbortCause::DescriptorContract {
                        code: "dependency-cache-failure-index-invalid",
                    })
            }
        };
        (cause, normalized_path)
    }
}

fn optional_eligible(cause: &ScopeAbortCause) -> bool {
    matches!(
        cause,
        ScopeAbortCause::Path(_)
            | ScopeAbortCause::AssetOpen(_)
            | ScopeAbortCause::AssetRead(_)
            | ScopeAbortCause::DependencyPolicy(_)
            | ScopeAbortCause::ResourceLimit { .. }
            | ScopeAbortCause::Allocation { .. }
            | ScopeAbortCause::Ledger(LedgerCause::LimitExceeded { .. })
    )
}

fn cacheable_dependency_cause(cause: &ScopeAbortCause) -> bool {
    matches!(
        cause,
        ScopeAbortCause::AssetOpen(_)
            | ScopeAbortCause::AssetRead(_)
            | ScopeAbortCause::ResourceLimit { .. }
            | ScopeAbortCause::Allocation { .. }
            | ScopeAbortCause::Ledger(LedgerCause::LimitExceeded { .. })
    )
}

pub(crate) fn read_opened_asset(
    coordinator: &RefCell<ImportCoordinator>,
    mut opened: OpenedAsset,
    ledger_phase: ImportLedgerPhase,
    progress_phase: ImportProgressPhase,
    dimension: ImportDimension,
    per_resource_limit: u64,
    aggregate_limit: u64,
) -> AssetSnapshotResult {
    coordinator
        .borrow_mut()
        .poll()
        .map_err(ScopeAbortCause::Control)?;
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
        charge_asset_attempted(coordinator, dimension, length)?;
        attempted_precharged = length;
        if length > effective_limit {
            return Err(ScopeAbortCause::ResourceLimit {
                resource: "asset-bytes",
                limit: effective_limit,
            });
        }
    }

    let mut bytes = Vec::new();
    let mut reservations = prepare_snapshot_reservation_list(coordinator, ledger_phase)?;
    let mut payload_reservation = None;
    let aggregate_capacity_limit = if exact_length.is_some() {
        u64::MAX
    } else {
        remaining_attempted_asset_bytes(
            coordinator,
            ImportDimension::TotalAssetBytes,
            aggregate_limit,
        )?
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
        reserve_snapshot_capacity(
            coordinator,
            &mut bytes,
            &mut payload_reservation,
            ledger_phase,
            dimension,
            capacity,
        )?;
    }

    let mut chunk = [0_u8; 16 * 1024];
    loop {
        coordinator
            .borrow_mut()
            .poll()
            .map_err(ScopeAbortCause::Control)?;
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
                coordinator,
                ImportDimension::TotalAssetBytes,
                aggregate_limit,
            )?;
            if aggregate_remaining == 0 {
                return Err(attempted_asset_limit_cause(
                    coordinator,
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
            charge_asset_attempted(coordinator, dimension, requested)?;
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
            charge_asset_consumed(coordinator, dimension, observed)?;
        }
        if exact_length.is_some() && after > attempted_precharged {
            charge_asset_attempted(coordinator, dimension, after - attempted_precharged)?;
            attempted_precharged = after;
        }
        if exact_length.is_some() && observed != 0 {
            charge_asset_consumed(coordinator, dimension, observed)?;
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
                coordinator,
                ImportDimension::TotalAssetBytes,
                aggregate_limit,
            )? == 0
        {
            return Err(attempted_asset_limit_cause(
                coordinator,
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
            reserve_snapshot_capacity(
                coordinator,
                &mut bytes,
                &mut payload_reservation,
                ledger_phase,
                dimension,
                count,
            )?;
        }
        bytes.extend_from_slice(&chunk[..count]);
        let mut coordinator = coordinator.borrow_mut();
        let completed = coordinator
            .dimension_evidence(dimension)
            .map_err(ScopeAbortCause::Ledger)?
            .consumed();
        coordinator
            .report_progress(
                progress_phase,
                ImportProgressUnit::Bytes,
                completed,
                progress_total,
            )
            .map_err(ScopeAbortCause::Control)?;
    }

    if let Some(reservation) = payload_reservation {
        reservations.push(reservation);
    }
    Ok((bytes, reservations))
}

fn charge_asset_attempted(
    coordinator: &RefCell<ImportCoordinator>,
    dimension: ImportDimension,
    amount: u64,
) -> Result<(), ScopeAbortCause> {
    let mut coordinator = coordinator.borrow_mut();
    let mut checkpoint = coordinator.checkpoint();
    checkpoint
        .charge_attempted(dimension, amount)
        .and_then(|()| checkpoint.charge_attempted(ImportDimension::TotalAssetBytes, amount))
        .map_err(ScopeAbortCause::Ledger)
}

fn remaining_attempted_asset_bytes(
    coordinator: &RefCell<ImportCoordinator>,
    dimension: ImportDimension,
    limit: u64,
) -> Result<u64, ScopeAbortCause> {
    let attempted = coordinator
        .borrow()
        .dimension_evidence(dimension)
        .map_err(ScopeAbortCause::Ledger)?
        .attempted();
    Ok(limit.saturating_sub(attempted))
}

fn attempted_asset_limit_cause(
    coordinator: &RefCell<ImportCoordinator>,
    dimension: ImportDimension,
    limit: u64,
) -> ScopeAbortCause {
    let attempted = coordinator
        .borrow()
        .dimension_evidence(dimension)
        .map_or(limit, |evidence| evidence.attempted());
    ScopeAbortCause::Ledger(LedgerCause::LimitExceeded {
        class: CounterClass::Attempted,
        dimension,
        value: attempted.saturating_add(1),
        limit,
    })
}

fn charge_asset_consumed(
    coordinator: &RefCell<ImportCoordinator>,
    dimension: ImportDimension,
    amount: u64,
) -> Result<(), ScopeAbortCause> {
    let mut coordinator = coordinator.borrow_mut();
    let mut checkpoint = coordinator.checkpoint();
    checkpoint
        .charge_consumed(dimension, amount)
        .and_then(|()| checkpoint.charge_consumed(ImportDimension::TotalAssetBytes, amount))
        .map_err(ScopeAbortCause::Ledger)
}

pub(crate) fn prepare_snapshot_reservation_list(
    coordinator: &RefCell<ImportCoordinator>,
    phase: ImportLedgerPhase,
) -> Result<Vec<ImportReservation>, ScopeAbortCause> {
    const RESERVATION_SLOTS: usize = 2;
    let plan = CapacityPlan::for_type::<ImportReservation>(0, 0, RESERVATION_SLOTS)
        .map_err(|cause| capacity_plan_cause("asset-snapshot-reservation-list", cause))?;
    let envelope = plan
        .envelope_delta_bytes()
        .map_err(|cause| capacity_plan_cause("asset-snapshot-reservation-list", cause))?;
    let mut reservation = coordinator
        .borrow_mut()
        .checkpoint()
        .reserve(phase, ImportDimension::ScratchBytes, envelope)
        .map_err(ScopeAbortCause::Ledger)?;
    let mut reservations = Vec::new();
    plan.verify_preflight(reservations.len(), reservations.capacity())
        .map_err(|cause| capacity_plan_cause("asset-snapshot-reservation-list", cause))?;
    reservations
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| ScopeAbortCause::Allocation {
            resource: "asset-snapshot-reservation-list",
        })?;
    let actual = plan
        .finish(reservations.len(), reservations.capacity())
        .map_err(|cause| capacity_plan_cause("asset-snapshot-reservation-list", cause))?;
    coordinator
        .borrow_mut()
        .checkpoint()
        .resize_reservation(&mut reservation, actual)
        .map_err(ScopeAbortCause::Ledger)?;
    reservations.push(reservation);
    Ok(reservations)
}

fn reserve_snapshot_capacity(
    coordinator: &RefCell<ImportCoordinator>,
    bytes: &mut Vec<u8>,
    payload_reservation: &mut Option<ImportReservation>,
    phase: ImportLedgerPhase,
    dimension: ImportDimension,
    additional: usize,
) -> Result<(), ScopeAbortCause> {
    let desired_capacity = bytes.len().checked_add(additional).ok_or_else(|| {
        capacity_plan_cause(
            "asset-snapshot-capacity",
            CapacityPlanError::CapacityOverflow,
        )
    })?;
    let plan = CapacityPlan::for_type::<u8>(bytes.len(), bytes.capacity(), desired_capacity)
        .map_err(|cause| capacity_plan_cause("asset-snapshot-capacity", cause))?;
    let envelope_delta = plan
        .envelope_delta_bytes()
        .map_err(|cause| capacity_plan_cause("asset-snapshot-capacity", cause))?;
    if envelope_delta == 0 {
        return Ok(());
    }
    let prior_bytes = payload_reservation
        .as_ref()
        .map_or(0, ImportReservation::bytes);
    let envelope = prior_bytes.checked_add(envelope_delta).ok_or_else(|| {
        capacity_plan_cause(
            "asset-snapshot-capacity",
            CapacityPlanError::CapacityOverflow,
        )
    })?;
    if let Some(reservation) = payload_reservation.as_mut() {
        coordinator
            .borrow_mut()
            .checkpoint()
            .resize_reservation(reservation, envelope)
            .map_err(ScopeAbortCause::Ledger)?;
    } else {
        *payload_reservation = Some(
            coordinator
                .borrow_mut()
                .checkpoint()
                .reserve(phase, dimension, envelope)
                .map_err(ScopeAbortCause::Ledger)?,
        );
    }
    plan.verify_preflight(bytes.len(), bytes.capacity())
        .map_err(|cause| capacity_plan_cause("asset-snapshot-capacity", cause))?;
    bytes
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| ScopeAbortCause::Allocation {
            resource: "asset-snapshot",
        })?;
    let actual_delta = plan
        .finish(bytes.len(), bytes.capacity())
        .map_err(|cause| capacity_plan_cause("asset-snapshot-capacity", cause))?;
    let actual = prior_bytes.checked_add(actual_delta).ok_or_else(|| {
        capacity_plan_cause(
            "asset-snapshot-capacity",
            CapacityPlanError::CapacityOverflow,
        )
    })?;
    if let Some(reservation) = payload_reservation.as_mut() {
        coordinator
            .borrow_mut()
            .checkpoint()
            .resize_reservation(reservation, actual)
            .map_err(ScopeAbortCause::Ledger)?;
    }
    Ok(())
}

fn capacity_plan_cause(resource: &'static str, cause: CapacityPlanError) -> ScopeAbortCause {
    match cause {
        CapacityPlanError::CapacityOverflow | CapacityPlanError::AllocationTooLarge { .. } => {
            ScopeAbortCause::ResourceLimit {
                resource,
                limit: u64::MAX,
            }
        }
        _ => ScopeAbortCause::DescriptorContract {
            code: "capacity-plan-contract-failed",
        },
    }
}

fn dependency_cache_capacity_cause(error: DependencyCacheCapacityError) -> ScopeAbortCause {
    match error {
        DependencyCacheCapacityError::Capacity { storage, cause } => {
            capacity_plan_cause(dependency_cache_storage_resource(storage), cause)
        }
        DependencyCacheCapacityError::Allocation { storage } => ScopeAbortCause::Allocation {
            resource: dependency_cache_storage_resource(storage),
        },
        DependencyCacheCapacityError::Stale => ScopeAbortCause::DescriptorContract {
            code: "dependency-cache-capacity-plan-stale",
        },
        DependencyCacheCapacityError::IndexContract => ScopeAbortCause::DescriptorContract {
            code: "dependency-cache-index-contract-failed",
        },
    }
}

const fn dependency_cache_storage_resource(storage: DependencyCacheStorage) -> &'static str {
    match storage {
        DependencyCacheStorage::Entries => "dependency-cache-entry-capacity",
        DependencyCacheStorage::Index => "dependency-cache-index-capacity",
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

/// Scene construction adapter that latches core builder failures in the scope.
///
/// Collection internals are intentionally unavailable to format implementations.
///
/// ```compile_fail
/// use baozi_core::NodeId;
/// use baozi_import::ScopedSceneBuilder;
///
/// fn bypass_capacity(builder: &mut ScopedSceneBuilder<'_, '_>, node: NodeId) {
///     let _ = builder.node_mut(node);
/// }
/// ```
pub struct ScopedSceneBuilder<'scope, 'session> {
    scope: &'scope ParseScope<'session>,
    builder: SceneBuilder,
    reservations: Vec<ImportReservation>,
    reservation_list_capacity: Option<ImportReservation>,
}

impl ScopedSceneBuilder<'_, '_> {
    /// Transfers live storage that becomes owned by this draft.
    pub fn adopt_reservation(&mut self, reservation: ImportLiveReservation<'_>) -> ParseResult<()> {
        self.scope.poll_active()?;
        self.reserve_reservation_slots(1)?;
        self.reservations.push(reservation.into_reservation());
        Ok(())
    }

    pub fn root(&self) -> NodeId {
        self.builder.root()
    }

    pub fn mesh_count(&self) -> usize {
        self.builder.mesh_count()
    }

    pub fn add_node(&mut self, node: Node) -> ParseResult<NodeId> {
        self.scope.poll_active()?;
        self.reserve_node_capacity(self.builder.root())?;
        self.builder
            .add_node(node)
            .map_err(|cause| self.scope.abort(ScopeAbortCause::SceneBuild(cause)))
    }

    pub fn add_child_node(&mut self, parent: NodeId, node: Node) -> ParseResult<NodeId> {
        self.scope.poll_active()?;
        self.reserve_node_capacity(parent)?;
        self.builder
            .add_child_node(parent, node)
            .map_err(|cause| self.scope.abort(ScopeAbortCause::SceneBuild(cause)))
    }

    pub fn add_mesh(&mut self, mesh: Mesh) -> ParseResult<MeshId> {
        self.scope.poll_active()?;
        self.reserve_collection_capacity(SceneCollection::Meshes)?;
        self.builder
            .add_mesh(mesh)
            .map_err(|cause| self.scope.abort(ScopeAbortCause::SceneBuild(cause)))
    }

    pub fn add_material(&mut self, material: Material) -> ParseResult<MaterialId> {
        self.scope.poll_active()?;
        self.reserve_collection_capacity(SceneCollection::Materials)?;
        self.builder
            .add_material(material)
            .map_err(|cause| self.scope.abort(ScopeAbortCause::SceneBuild(cause)))
    }

    pub fn add_texture(&mut self, texture: Texture) -> ParseResult<TextureId> {
        self.scope.poll_active()?;
        self.reserve_collection_capacity(SceneCollection::Textures)?;
        self.builder
            .add_texture(texture)
            .map_err(|cause| self.scope.abort(ScopeAbortCause::SceneBuild(cause)))
    }

    pub fn add_animation(&mut self, animation: Animation) -> ParseResult<AnimationId> {
        self.scope.poll_active()?;
        self.reserve_collection_capacity(SceneCollection::Animations)?;
        self.builder
            .add_animation(animation)
            .map_err(|cause| self.scope.abort(ScopeAbortCause::SceneBuild(cause)))
    }

    pub fn add_camera(&mut self, camera: Camera) -> ParseResult<CameraId> {
        self.scope.poll_active()?;
        self.reserve_collection_capacity(SceneCollection::Cameras)?;
        self.builder
            .add_camera(camera)
            .map_err(|cause| self.scope.abort(ScopeAbortCause::SceneBuild(cause)))
    }

    pub fn add_light(&mut self, light: Light) -> ParseResult<LightId> {
        self.scope.poll_active()?;
        self.reserve_collection_capacity(SceneCollection::Lights)?;
        self.builder
            .add_light(light)
            .map_err(|cause| self.scope.abort(ScopeAbortCause::SceneBuild(cause)))
    }

    pub fn add_skin(&mut self, skin: Skin) -> ParseResult<SkinId> {
        self.scope.poll_active()?;
        self.reserve_collection_capacity(SceneCollection::Skins)?;
        self.builder
            .add_skin(skin)
            .map_err(|cause| self.scope.abort(ScopeAbortCause::SceneBuild(cause)))
    }

    pub fn node_mesh_binding_count(&mut self, id: NodeId) -> Option<usize> {
        if self.scope.preflight_active().is_err() {
            return None;
        }
        self.builder
            .node_mut(id)
            .map(|node| node.mesh_bindings.len())
    }

    pub fn add_node_mesh_binding(&mut self, id: NodeId, binding: MeshBinding) -> ParseResult<()> {
        self.scope.poll_active()?;
        let (len, capacity) = self
            .builder
            .node_mut(id)
            .map(|node| (node.mesh_bindings.len(), node.mesh_bindings.capacity()))
            .ok_or_else(|| {
                self.scope.abort(ScopeAbortCause::DescriptorContract {
                    code: "node-mesh-binding-node-out-of-range",
                })
            })?;
        let desired_capacity = len.checked_add(1).ok_or_else(|| {
            self.scope
                .resource_limit_exceeded("node-mesh-binding-capacity", u64::MAX)
        })?;
        let plan = CapacityPlan::for_type::<MeshBinding>(len, capacity, desired_capacity).map_err(
            |cause| {
                self.scope
                    .capacity_plan_error("node-mesh-binding-capacity", cause)
            },
        )?;
        let envelope = plan.envelope_delta_bytes().map_err(|cause| {
            self.scope
                .capacity_plan_error("node-mesh-binding-capacity", cause)
        })?;
        if envelope == 0 {
            self.builder
                .node_mut(id)
                .ok_or_else(|| {
                    self.scope.abort(ScopeAbortCause::DescriptorContract {
                        code: "node-mesh-binding-node-out-of-range",
                    })
                })?
                .mesh_bindings
                .push(binding);
            return Ok(());
        }

        self.reserve_reservation_slots(1)?;
        let mut reservation = self
            .scope
            .reserve_live(ImportStorageClass::SceneConstruction, envelope)?;
        let node = self.builder.node_mut(id).ok_or_else(|| {
            self.scope.abort(ScopeAbortCause::DescriptorContract {
                code: "node-mesh-binding-node-out-of-range",
            })
        })?;
        plan.verify_preflight(node.mesh_bindings.len(), node.mesh_bindings.capacity())
            .map_err(|cause| {
                self.scope
                    .capacity_plan_error("node-mesh-binding-capacity", cause)
            })?;
        node.mesh_bindings
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| self.scope.allocation_failure("node-mesh-binding-capacity"))?;
        let actual = plan
            .finish(node.mesh_bindings.len(), node.mesh_bindings.capacity())
            .map_err(|cause| {
                self.scope
                    .capacity_plan_error("node-mesh-binding-capacity", cause)
            })?;
        self.scope.resize_live(&mut reservation, actual)?;
        self.builder
            .node_mut(id)
            .ok_or_else(|| {
                self.scope.abort(ScopeAbortCause::DescriptorContract {
                    code: "node-mesh-binding-node-out-of-range",
                })
            })?
            .mesh_bindings
            .push(binding);
        if actual != 0 {
            self.reservations.push(reservation.into_reservation());
        }
        Ok(())
    }

    pub fn set_node_mesh_binding_skin(
        &mut self,
        node: NodeId,
        binding_index: usize,
        skin: SkinId,
    ) -> bool {
        if self.scope.poll_active().is_err() {
            return false;
        }
        let Some(binding) = self
            .builder
            .node_mut(node)
            .and_then(|node| node.mesh_bindings.get_mut(binding_index))
        else {
            return false;
        };
        binding.skin = Some(skin);
        true
    }

    pub fn set_space(&mut self, space: SceneSpace) -> ParseResult<()> {
        self.scope.poll_active()?;
        *self.builder.space_mut() = space;
        Ok(())
    }

    pub fn insert_metadata(
        &mut self,
        key: String,
        value: MetadataValue,
    ) -> ParseResult<Option<MetadataValue>> {
        self.scope.poll_active()?;
        let additional = usize::from(!self.builder.metadata_mut().contains_key(key.as_str()));
        let plan = self
            .builder
            .metadata_mut()
            .capacity_plan(additional)
            .map_err(|cause| self.metadata_error(cause))?;
        let admitted = plan
            .envelope_delta_bytes()
            .map_err(|cause| self.metadata_error(cause))?;
        self.reserve_reservation_slots(usize::from(admitted != 0))?;
        let token = self.reserve_capacity_token(admitted)?;
        let actual = self
            .builder
            .metadata_mut()
            .apply_capacity_plan(plan)
            .map_err(|cause| self.metadata_error(cause))?;
        self.retain_capacity(token, admitted, actual)?;
        self.builder
            .metadata_mut()
            .insert_reserved(key, value)
            .map_err(|cause| self.metadata_error(cause))
    }

    fn metadata_error(&self, cause: OrderedMapError) -> ParseFailure {
        match cause {
            OrderedMapError::CapacityOverflow => self
                .scope
                .resource_limit_exceeded("scene-metadata-capacity", u64::MAX),
            OrderedMapError::AllocationFailed => {
                self.scope.allocation_failure("scene-metadata-capacity")
            }
            OrderedMapError::CapacityNotReserved => {
                self.scope.abort(ScopeAbortCause::DescriptorContract {
                    code: "scene-metadata-capacity-not-reserved",
                })
            }
            _ => self.scope.abort(ScopeAbortCause::DescriptorContract {
                code: "scene-metadata-capacity-error-unknown",
            }),
        }
    }

    fn reserve_collection_capacity(&mut self, collection: SceneCollection) -> ParseResult<()> {
        self.scope.preflight_active()?;
        let plan = self
            .builder
            .collection_capacity_plan(collection, 1)
            .map_err(|cause| self.scope.abort(ScopeAbortCause::SceneBuild(cause)))?;
        let admitted = plan
            .envelope_delta_bytes()
            .map_err(|cause| self.scope.abort(ScopeAbortCause::SceneBuild(cause)))?;
        self.reserve_reservation_slots(usize::from(admitted != 0))?;
        let token = self.reserve_capacity_token(admitted)?;
        let actual = self
            .builder
            .apply_collection_capacity_plan(collection, plan)
            .map_err(|cause| self.scope.abort(ScopeAbortCause::SceneBuild(cause)))?;
        self.retain_capacity(token, admitted, actual)
    }

    fn reserve_node_capacity(&mut self, parent: NodeId) -> ParseResult<()> {
        self.scope.preflight_active()?;
        let node_plan = self
            .builder
            .collection_capacity_plan(SceneCollection::Nodes, 1)
            .map_err(|cause| self.scope.abort(ScopeAbortCause::SceneBuild(cause)))?;
        let node_admitted = node_plan
            .envelope_delta_bytes()
            .map_err(|cause| self.scope.abort(ScopeAbortCause::SceneBuild(cause)))?;
        let child_plan = self
            .builder
            .node_children_capacity_plan(parent, 1)
            .map_err(|cause| self.scope.abort(ScopeAbortCause::SceneBuild(cause)))?;
        let child_admitted = child_plan
            .envelope_delta_bytes()
            .map_err(|cause| self.scope.abort(ScopeAbortCause::SceneBuild(cause)))?;
        let token_count = usize::from(node_admitted != 0) + usize::from(child_admitted != 0);
        self.reserve_reservation_slots(token_count)?;
        let node_token = self.reserve_capacity_token(node_admitted)?;
        let child_token = self.reserve_capacity_token(child_admitted)?;

        let node_actual = self
            .builder
            .apply_collection_capacity_plan(SceneCollection::Nodes, node_plan)
            .map_err(|cause| self.scope.abort(ScopeAbortCause::SceneBuild(cause)))?;
        self.retain_capacity(node_token, node_admitted, node_actual)?;
        let child_actual = self
            .builder
            .apply_node_children_capacity_plan(parent, child_plan)
            .map_err(|cause| self.scope.abort(ScopeAbortCause::SceneBuild(cause)))?;
        self.retain_capacity(child_token, child_admitted, child_actual)
    }

    fn reserve_reservation_slots(&mut self, payload_slots: usize) -> ParseResult<()> {
        self.scope.preflight_active()?;
        if payload_slots == 0 {
            return Ok(());
        }
        let desired_capacity = self
            .reservations
            .len()
            .checked_add(payload_slots)
            .ok_or_else(|| {
                self.scope
                    .resource_limit_exceeded("scene-builder-reservation-list", u64::MAX)
            })?;
        let plan = CapacityPlan::for_type::<ImportReservation>(
            self.reservations.len(),
            self.reservations.capacity(),
            desired_capacity,
        )
        .map_err(|cause| {
            self.scope
                .capacity_plan_error("scene-builder-reservation-list", cause)
        })?;
        let envelope_delta = plan.envelope_delta_bytes().map_err(|cause| {
            self.scope
                .capacity_plan_error("scene-builder-reservation-list", cause)
        })?;
        if envelope_delta == 0 {
            return Ok(());
        }
        let prior_reservation = self.reservation_list_capacity.take();
        let prior_bytes = prior_reservation
            .as_ref()
            .map_or(0, ImportReservation::bytes);
        let envelope = prior_bytes.checked_add(envelope_delta).ok_or_else(|| {
            self.scope
                .resource_limit_exceeded("scene-builder-reservation-list", u64::MAX)
        })?;
        let mut reservation = match prior_reservation {
            Some(mut reservation) => {
                self.scope
                    .coordinator
                    .try_borrow_mut()
                    .map_err(|_| self.scope.reentrant_capability_failure())?
                    .checkpoint()
                    .resize_reservation(&mut reservation, envelope)
                    .map_err(|cause| self.scope.abort(ScopeAbortCause::Ledger(cause)))?;
                reservation
            }
            None => self
                .scope
                .coordinator
                .try_borrow_mut()
                .map_err(|_| self.scope.reentrant_capability_failure())?
                .checkpoint()
                .reserve(
                    ImportLedgerPhase::SceneConstruction,
                    ImportDimension::ScratchBytes,
                    envelope,
                )
                .map_err(|cause| self.scope.abort(ScopeAbortCause::Ledger(cause)))?,
        };
        plan.verify_preflight(self.reservations.len(), self.reservations.capacity())
            .map_err(|cause| {
                self.scope
                    .capacity_plan_error("scene-builder-reservation-list", cause)
            })?;
        self.reservations
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| {
                self.scope
                    .allocation_failure("scene-builder-reservation-list")
            })?;
        let actual_delta = plan
            .finish(self.reservations.len(), self.reservations.capacity())
            .map_err(|cause| {
                self.scope
                    .capacity_plan_error("scene-builder-reservation-list", cause)
            })?;
        let actual = prior_bytes.checked_add(actual_delta).ok_or_else(|| {
            self.scope
                .resource_limit_exceeded("scene-builder-reservation-list", u64::MAX)
        })?;
        self.scope
            .coordinator
            .try_borrow_mut()
            .map_err(|_| self.scope.reentrant_capability_failure())?
            .checkpoint()
            .resize_reservation(&mut reservation, actual)
            .map_err(|cause| self.scope.abort(ScopeAbortCause::Ledger(cause)))?;
        self.reservation_list_capacity = Some(reservation);
        Ok(())
    }

    fn reserve_capacity_token(&self, admitted: u64) -> ParseResult<Option<ImportReservation>> {
        self.scope.preflight_active()?;
        if admitted == 0 {
            return Ok(None);
        }
        self.scope
            .reserve_live(ImportStorageClass::SceneConstruction, admitted)
            .map(|reservation| Some(reservation.into_reservation()))
    }

    fn retain_capacity(
        &mut self,
        token: Option<ImportReservation>,
        admitted: u64,
        actual: u64,
    ) -> ParseResult<()> {
        self.scope.preflight_active()?;
        if actual > admitted {
            return Err(self.scope.abort(ScopeAbortCause::DescriptorContract {
                code: "scene-builder-capacity-reservation-underestimated",
            }));
        }
        let Some(mut token) = token else {
            if actual == 0 {
                return Ok(());
            }
            return Err(self.scope.abort(ScopeAbortCause::DescriptorContract {
                code: "scene-builder-capacity-growth-without-reservation",
            }));
        };
        if actual == 0 {
            return Ok(());
        }
        self.scope
            .coordinator
            .try_borrow_mut()
            .map_err(|_| self.scope.reentrant_capability_failure())?
            .checkpoint()
            .resize_reservation(&mut token, actual)
            .map_err(|cause| self.scope.abort(ScopeAbortCause::Ledger(cause)))?;
        self.reservations.push(token);
        Ok(())
    }

    pub fn finish_draft(mut self) -> ParseResult<SceneDraft> {
        self.scope.poll_active()?;
        let footprint = self
            .builder
            .storage_footprint()
            .map_err(|cause| self.scope.abort(ScopeAbortCause::SceneAccounting(cause)))?
            .accounted_payload_bytes();
        self.scope
            .seal_draft_reservations(&mut self.reservations, footprint)?;
        Ok(self.builder.finish_draft())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{ImportOperationControl, import_ledger_limits};
    use baozi_io::{AssetLength, AuthorityDomain};
    use std::io::Cursor;

    fn assert_not_send_sync<T: ?Sized>() {
        let _ = std::any::type_name::<T>();
    }

    #[test]
    fn parse_scope_is_intentionally_session_local() {
        assert_not_send_sync::<ParseScope<'_>>();
    }

    #[test]
    fn optional_eligibility_excludes_control_and_extension_failures() {
        assert!(optional_eligible(&ScopeAbortCause::ResourceLimit {
            resource: "bytes",
            limit: 1,
        }));
        assert!(!optional_eligible(&ScopeAbortCause::TrustedExtension {
            boundary: TrustedExtensionBoundary::Importer,
            code: "panic",
        }));
    }

    #[test]
    fn opened_asset_snapshot_is_unique_metered_vec_storage() {
        let limits = ResourceLimits::default();
        let coordinator = RefCell::new(ImportCoordinator::new(
            ImportOperationControl::default(),
            import_ledger_limits(&limits).expect("valid import ledger limits"),
        ));
        let payload = b"metered snapshot".to_vec();
        let opened = OpenedAsset::new(
            AuthorityDomain::new(),
            AssetPath::new("scene.bin").unwrap(),
            Cursor::new(payload.clone()),
            Some(AssetLength::Exact(payload.len() as u64)),
        );

        let (snapshot, reservations) = read_opened_asset(
            &coordinator,
            opened,
            ImportLedgerPhase::PrimarySnapshot,
            ImportProgressPhase::AcquirePrimary,
            ImportDimension::PrimaryBytes,
            limits.max_primary_asset_bytes,
            limits.max_total_asset_bytes,
        )
        .unwrap();

        assert_eq!(snapshot, payload);
        assert_eq!(reservations.len(), 2);
        assert!(reservations.capacity() >= reservations.len());
        assert!(
            coordinator
                .borrow()
                .try_ledger_snapshot()
                .expect("active test ledger snapshot")
                .current_live_bytes()
                > 0
        );

        drop(snapshot);
        drop(reservations);
        assert_eq!(
            coordinator
                .borrow()
                .try_ledger_snapshot()
                .expect("active test ledger snapshot")
                .current_live_bytes(),
            0
        );
    }
}
