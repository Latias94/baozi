use crate::config::{ConformancePolicy, ImportLossKind, LossPolicy};
use crate::dependency::{
    AuthorizedDependency, DependencyCache, DependencyCacheCapacityError,
    DependencyCacheDisposition, DependencyCacheEntry, DependencyCacheStorage, DependencyIdentity,
    DependencyReference, DependencyRequirement, DependencyResolver, DependencyRoleId,
    DependencySnapshot, OptionalDependencyEvidence, OptionalDependencyOutcome,
    SharedDependencySnapshot, shared_dependency_snapshot_allocation_bytes,
};
use crate::descriptor::TextDecodingMode;
use crate::diagnostic::{
    DiagnosticByteRange, DiagnosticCause, DiagnosticCode, DiagnosticDetailSink,
    DiagnosticDropReason, DiagnosticLineColumn, DiagnosticPrepareCause, DiagnosticSeverity,
    DiagnosticSink, DiagnosticSource, PreparedDiagnostic,
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
use crate::loss::{ImportLossDisposition, ImportLossEvidence, ImportLossSubject};
use crate::metered_storage::{
    AccountedReservationVec, AccountedVec, MeteredDecodedText, MeteredMetadataMap,
    MeteredOpaqueAllowance, MeteredOrderedMap, MeteredSceneVec, MeteredString, MeteredVec,
    PreparedSceneValue, SceneValueParts, SceneValueReservations, StorageMeter,
};
use crate::options::{CompiledFormatOptions, FormatOptionKey};
use crate::registry::RegistryEntry;
use crate::source_scope::{
    FinalizedSourceScopeRecord, LiveSourceScopeRecord, SourceScopeRecordBuilder,
};
use baozi_core::{
    Aabb, AlphaMode, Animation, AnimationId, Camera, CameraId, CameraProjection, Color, ColorSpace,
    Light, LightId, Mat4, Material, MaterialId, Mesh, MeshBinding, MeshId, MetadataMap,
    MetadataValue, Node, NodeId, OrderedMap, OrderedMapError, PreparedSceneInsert,
    PrimitiveTopology, SceneBuildError, SceneBuildValue, SceneBuilder, SceneCapacityPlan,
    SceneCollection, SceneDraft, SceneSpace, SceneStorageFootprint, SceneValueFootprint,
    ShadingModel, Skin, SkinId, Texture, TextureId, TextureRole, TextureSampler, TextureSlot,
    TextureSource, TextureTransform, Vec2, Vec3, Vec4, VertexAttribute, VertexAttributeData,
    VertexAttributeSemantic,
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
type AssetSnapshotResult = Result<AccountedVec<u8>, ScopeAbortCause>;

/// Fixed-size fields used when assembling a metered mesh value.
#[derive(Debug, Clone, Copy)]
pub struct MeshBuildConfig {
    pub topology: PrimitiveTopology,
    pub material: Option<MaterialId>,
    pub bounds: Option<Aabb>,
}

/// Fixed-size fields used when assembling a metered material value.
#[derive(Debug, Clone, Copy)]
pub struct MaterialBuildConfig {
    pub shading_model: ShadingModel,
    pub base_color: Color,
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: Color,
    pub alpha_mode: AlphaMode,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
}

/// Fixed-size fields used when assembling a metered node value.
#[derive(Debug, Clone, Copy)]
pub struct NodeBuildConfig {
    pub transform: Mat4,
    pub camera: Option<CameraId>,
    pub light: Option<LightId>,
}

/// Fixed-size fields used when assembling a metered texture value.
#[derive(Debug, Clone, Copy)]
pub struct TextureBuildConfig {
    pub sampler: TextureSampler,
}

/// Fixed-size fields used when assembling a metered texture-slot value.
#[derive(Debug, Clone, Copy)]
pub struct TextureSlotBuildConfig {
    pub texture: TextureId,
    pub role: TextureRole,
    pub color_space: ColorSpace,
    pub uv_set: u32,
    pub scale: f32,
    pub transform: TextureTransform,
}

/// Fixed-size fields used when assembling a metered skin value.
#[derive(Debug, Clone, Copy)]
pub struct SkinBuildConfig {
    pub skeleton_root: Option<NodeId>,
}

/// Allocation-free fields used when assembling a metered camera value.
#[derive(Debug, Clone)]
pub struct CameraBuildConfig {
    pub projection: CameraProjection,
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
    _source_reservation: Option<ImportReservation>,
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
    resolver: &'session dyn DependencyResolver,
    conformance_policy: ConformancePolicy,
    loss_policy: LossPolicy,
    limits: &'session ResourceLimits,
    coordinator: &'session RefCell<ImportCoordinator>,
    diagnostics: &'session RefCell<DiagnosticSink>,
    diagnostic_line_indexes: &'session RefCell<AccountedVec<DiagnosticLineIndex>>,
    dependencies: &'session RefCell<DependencyCache>,
    optional_dependency_evidence: &'session RefCell<AccountedVec<OptionalDependencyEvidence>>,
    loss_evidence: &'session RefCell<AccountedVec<ImportLossEvidence>>,
    source_scope_record: &'session RefCell<Option<LiveSourceScopeRecord>>,
    draft_reservations: &'session RefCell<AccountedVec<ImportReservation>>,
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
    pub diagnostic_line_indexes: &'session RefCell<AccountedVec<DiagnosticLineIndex>>,
    pub dependencies: &'session RefCell<DependencyCache>,
    pub optional_dependency_evidence: &'session RefCell<AccountedVec<OptionalDependencyEvidence>>,
    pub loss_evidence: &'session RefCell<AccountedVec<ImportLossEvidence>>,
    pub source_scope_record: &'session RefCell<Option<LiveSourceScopeRecord>>,
    pub draft_reservations: &'session RefCell<AccountedVec<ImportReservation>>,
    pub format_failure_storage: &'session RefCell<Option<ImportReservation>>,
    pub terminal: &'session RefCell<Option<ScopeAbortCause>>,
}

struct PreparedDiagnosticEvent {
    prepared: PreparedDiagnostic,
    payload_reservation: Option<ImportReservation>,
    structural_reservation: Option<ImportReservation>,
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
            conformance_policy: parts.conformance_policy,
            loss_policy: parts.loss_policy,
            limits: parts.limits,
            coordinator: parts.coordinator,
            diagnostics: parts.diagnostics,
            diagnostic_line_indexes: parts.diagnostic_line_indexes,
            dependencies: parts.dependencies,
            optional_dependency_evidence: parts.optional_dependency_evidence,
            loss_evidence: parts.loss_evidence,
            source_scope_record: parts.source_scope_record,
            draft_reservations: parts.draft_reservations,
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

    /// Creates a dynamic canonical collection whose element payloads remain coupled to the outer
    /// collection until it is finished.
    pub fn metered_scene_vec<T>(&self, resource: &'static str) -> MeteredSceneVec<'_, T> {
        MeteredSceneVec {
            values: self.metered_vec(ImportStorageClass::SceneConstruction, resource),
            reservations: SceneValueReservations::new(),
        }
    }

    /// Creates canonical metadata storage with per-entry reservation ownership.
    pub fn metered_metadata_map(&self, resource: &'static str) -> MeteredMetadataMap<'_> {
        MeteredMetadataMap {
            values: self.metered_ordered_map(ImportStorageClass::SceneConstruction, resource),
            entries: self.metered_vec(ImportStorageClass::SceneConstruction, resource),
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

    /// Prepares a canonical value with no external metered parts.
    ///
    /// Values with retained dynamic storage are rejected because the empty reservation inventory
    /// cannot prove their allocation ownership.
    pub fn prepare_scene_value<T: SceneBuildValue>(
        &self,
        value: T,
    ) -> ParseResult<PreparedSceneValue<T>> {
        validate_prepared_scene_value(
            self,
            PreparedSceneValue {
                value: Some(value),
                reservations: SceneValueReservations::new(),
            },
        )
    }

    fn storage_meter(&self) -> StorageMeter<'_> {
        StorageMeter {
            coordinator: self.coordinator,
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
    ) -> Result<ImportReservation, ScopeAbortCause> {
        let (phase, dimension) = storage.accounting();
        self.coordinator
            .try_borrow_mut()
            .map_err(|_| ScopeAbortCause::DescriptorContract {
                code: "parse-capability-reentered",
            })?
            .checkpoint()
            .reserve(phase, dimension, bytes)
            .map_err(ScopeAbortCause::Ledger)
    }

    fn resize_storage_reservation(
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

    fn resize_live(
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
        self.coordinator
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .checkpoint()
            .charge_attempted(ImportDimension::LossEvidenceCount, 1)
            .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))?;
        let source_envelope = u64::try_from(source.path().as_str().len())
            .map_err(|_| self.resource_limit_exceeded("loss-evidence-source", u64::MAX))?;
        let mut reservation = self
            .coordinator
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .checkpoint()
            .reserve(
                ImportLedgerPhase::FormatNormalization,
                ImportDimension::LossEvidenceBytes,
                source_envelope,
            )
            .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))?;

        let retained_source = source
            .path()
            .try_clone()
            .map_err(|_| self.allocation_failure("loss-evidence-source"))?;
        let source_bytes = u64::try_from(retained_source.capacity_bytes())
            .map_err(|_| self.resource_limit_exceeded("loss-evidence-source", u64::MAX))?;
        if source_bytes != source_envelope {
            self.coordinator
                .try_borrow_mut()
                .map_err(|_| self.reentrant_capability_failure())?
                .checkpoint()
                .resize_reservation(&mut reservation, source_bytes)
                .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))?;
        }

        let (prior_capacity_bytes, capacity_result) = {
            let mut evidence = self
                .loss_evidence
                .try_borrow_mut()
                .map_err(|_| self.reentrant_capability_failure())?;
            let prior_capacity_bytes = evidence.capacity_bytes();
            let result = evidence.try_reserve(1, "loss-evidence-capacity", |capacity_bytes| {
                self.coordinator
                    .try_borrow_mut()
                    .map_err(|_| ScopeAbortCause::DescriptorContract {
                        code: "parse-capability-reentered",
                    })?
                    .checkpoint()
                    .reserve(
                        ImportLedgerPhase::FormatNormalization,
                        ImportDimension::LossEvidenceBytes,
                        capacity_bytes,
                    )
                    .map_err(ScopeAbortCause::Ledger)
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
        let charge = {
            let mut coordinator = self
                .coordinator
                .try_borrow_mut()
                .map_err(|_| self.reentrant_capability_failure())?;
            let mut checkpoint = coordinator.checkpoint();
            checkpoint.charge_attempted(ImportDimension::LossEvidenceBytes, actual_bytes)
        };
        if let Err(cause) = charge {
            return Err(self.abort(ScopeAbortCause::Ledger(cause)));
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
        let mut coordinator = self
            .coordinator
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?;
        let mut checkpoint = coordinator.checkpoint();
        checkpoint
            .charge_consumed_batch(&[
                (ImportDimension::LossEvidenceCount, 1),
                (ImportDimension::LossEvidenceBytes, stored_bytes),
            ])
            .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))
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
                .coordinator
                .try_borrow_mut()
                .map_err(|_| ScopeAbortCause::DescriptorContract {
                    code: "parse-capability-reentered",
                })
                .and_then(|mut coordinator| {
                    coordinator
                        .resize_reservation(&mut exact, payload_capacity_bytes)
                        .map_err(ScopeAbortCause::Ledger)
                });
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
                .coordinator
                .try_borrow_mut()
                .map_err(|_| ScopeAbortCause::DescriptorContract {
                    code: "parse-capability-reentered",
                })
                .and_then(|mut coordinator| {
                    coordinator
                        .resize_reservation(&mut structural_exact, structural_bytes)
                        .map_err(ScopeAbortCause::Ledger)
                });
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
        let mut coordinator = self
            .coordinator
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?;
        let mut checkpoint = coordinator.checkpoint();
        checkpoint
            .charge_consumed_batch(&[
                (ImportDimension::DiagnosticCount, 1),
                (ImportDimension::DiagnosticBytes, stored_bytes),
            ])
            .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))
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
    ) -> Result<ImportReservation, ScopeAbortCause> {
        self.coordinator
            .try_borrow_mut()
            .map_err(|_| ScopeAbortCause::DescriptorContract {
                code: "parse-capability-reentered",
            })?
            .checkpoint()
            .reserve(
                ImportLedgerPhase::FormatNormalization,
                ImportDimension::ScratchBytes,
                capacity_bytes,
            )
            .map_err(ScopeAbortCause::Ledger)
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
                    line_starts.try_push_reserved(start).map_err(|_| {
                        self.abort(ScopeAbortCause::DescriptorContract {
                            code: "diagnostic-line-index-slots-not-admitted",
                        })
                    })?;
                }
            }
            offset = offset.checked_add(chunk.len()).ok_or_else(|| {
                self.resource_limit_exceeded("diagnostic-line-index-offset", u64::MAX)
            })?;
        }
        permit.complete()?;

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
        let inline_bytes = u64::try_from(size_of::<SceneBuilder>())
            .map_err(|_| self.allocation_failure("scene-builder-inline-accounting"))?;
        let inline = self.reserve_live(ImportStorageClass::SceneConstruction, inline_bytes)?;
        let nodes = self.reserve_live(ImportStorageClass::SceneConstruction, capacity_bytes)?;
        let builder = SceneBuilder::try_new_with_capacity_plan(capacity_plan)
            .map_err(|cause| self.abort(ScopeAbortCause::SceneBuild(cause)))?;
        let mut node_slots = AccountedVec::new();
        node_slots
            .try_reserve(1, "scene-node-reservation-slots", |bytes| {
                reserve_scene_inventory_capacity(self, bytes)
            })
            .map_err(|cause| self.abort(cause))?;
        node_slots
            .try_push_reserved(NodeReservationSlots::default())
            .map_err(|_| {
                self.abort(ScopeAbortCause::DescriptorContract {
                    code: "scene-node-reservation-slot-not-admitted",
                })
            })?;

        Ok(ScopedSceneBuilder {
            scope: self,
            storage: AccountedSceneStorage {
                builder,
                inventory: SceneReservationInventory::new(
                    inline.into_reservation(),
                    nodes.into_reservation(),
                    node_slots,
                ),
            },
        })
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

        let acquired = self
            .open_and_read_dependency(io, &identity)
            .and_then(|bytes| {
                let snapshot_bytes = self.share_dependency_snapshot(bytes)?;
                let (snapshot_identity, identity_path_reservation) =
                    self.clone_dependency_identity(&identity)?;
                Ok((snapshot_bytes, snapshot_identity, identity_path_reservation))
            });
        match acquired {
            Ok((snapshot_bytes, snapshot_identity, identity_path_reservation)) => {
                if !self
                    .dependencies
                    .borrow_mut()
                    .store_available(&identity, Arc::clone(&snapshot_bytes))
                {
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
        bytes: AccountedVec<u8>,
    ) -> Result<Arc<SharedDependencySnapshot>, ScopeAbortCause> {
        let control_bytes = shared_dependency_snapshot_allocation_bytes().ok_or(
            ScopeAbortCause::ResourceLimit {
                resource: "dependency-snapshot-sharing",
                limit: u64::MAX,
            },
        )?;
        let reservation =
            self.reserve_dependency_storage_cause(u64::try_from(control_bytes).map_err(|_| {
                ScopeAbortCause::ResourceLimit {
                    resource: "dependency-snapshot-sharing",
                    limit: u64::MAX,
                }
            })?)?;
        Ok(Arc::new(SharedDependencySnapshot::new(bytes, reservation)))
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
        let retained_evidence = self.clone_optional_dependency_evidence(
            failure.identity.as_ref(),
            normalized_path.path(),
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

    pub(crate) fn source_scope_preflight(&self) -> ParseResult<()> {
        self.preflight_active()
    }

    pub(crate) fn source_scope_abort(&self, cause: ScopeAbortCause) -> ParseFailure {
        self.abort(cause)
    }

    pub(crate) fn source_scope_charge_batch(
        &self,
        charges: &[(ImportDimension, u64)],
    ) -> ParseResult<()> {
        self.preflight_active()?;
        let mut coordinator = self
            .coordinator
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?;
        let mut checkpoint = coordinator.checkpoint();
        checkpoint
            .charge_observed_batch(charges)
            .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))
    }

    fn source_scope_charge_attempted(
        &self,
        dimension: ImportDimension,
        amount: u64,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        self.coordinator
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .checkpoint()
            .charge_attempted(dimension, amount)
            .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))
    }

    fn source_scope_charge_consumed(
        &self,
        dimension: ImportDimension,
        amount: u64,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        self.coordinator
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?
            .checkpoint()
            .charge_consumed(dimension, amount)
            .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))
    }

    fn charge_source_scope_bundle_consumed(
        &self,
        loss_bytes: u64,
        diagnostic_bytes: u64,
    ) -> ParseResult<()> {
        self.preflight_active()?;
        let mut coordinator = self
            .coordinator
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?;
        let mut checkpoint = coordinator.checkpoint();
        checkpoint
            .charge_consumed_batch(&[
                (ImportDimension::DiagnosticCount, 1),
                (ImportDimension::DiagnosticBytes, diagnostic_bytes),
                (ImportDimension::LossEvidenceCount, 1),
                (ImportDimension::LossEvidenceBytes, loss_bytes),
                (ImportDimension::SourceScopeRecordCount, 1),
            ])
            .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))
    }

    fn reentrant_capability_failure(&self) -> ParseFailure {
        self.abort(ScopeAbortCause::DescriptorContract {
            code: "parse-capability-reentered",
        })
    }

    fn seal_draft_inventory(
        &self,
        inventory: &mut SceneReservationInventory,
        required_bytes: u64,
    ) -> ParseResult<()> {
        let snapshot = inventory
            .snapshot()
            .map_err(|code| self.abort(ScopeAbortCause::DescriptorContract { code }))?;
        if snapshot.bytes < required_bytes {
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "draft-storage-was-not-reserved-before-allocation",
            }));
        }

        let typed_count = inventory
            .typed_token_count()
            .map_err(|code| self.abort(ScopeAbortCause::DescriptorContract { code }))?;
        inventory
            .payload
            .try_reserve(
                typed_count,
                "scene-reservation-flattening",
                |capacity_bytes| reserve_scene_inventory_capacity(self, capacity_bytes),
            )
            .map_err(|cause| self.abort(cause))?;

        let mut session_reservations = self
            .draft_reservations
            .try_borrow_mut()
            .map_err(|_| self.reentrant_capability_failure())?;
        session_reservations
            .try_reserve(
                snapshot.token_count,
                "draft-reservation-side-channel",
                |capacity_bytes| {
                    self.coordinator
                        .try_borrow_mut()
                        .map_err(|_| ScopeAbortCause::DescriptorContract {
                            code: "parse-capability-reentered",
                        })?
                        .checkpoint()
                        .reserve(
                            ImportLedgerPhase::SceneConstruction,
                            ImportDimension::ScratchBytes,
                            capacity_bytes,
                        )
                        .map_err(ScopeAbortCause::Ledger)
                },
            )
            .map_err(|cause| self.abort(cause))?;
        if inventory.transfer_into(&mut session_reservations).is_err() {
            return Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "draft-reservation-slots-not-admitted",
            }));
        }
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

fn metered_copy_chunk_len<T>() -> usize {
    METERED_COPY_POLL_INTERVAL
        .checked_div(size_of::<T>())
        .unwrap_or(METERED_COPY_POLL_INTERVAL)
        .max(1)
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
        if self.terminal.borrow().is_some() {
            Err(ParseFailure::scope())
        } else {
            Ok(())
        }
    }

    fn poll(&self) -> ParseResult<()> {
        self.preflight_active()?;
        let result = self
            .coordinator
            .try_borrow_mut()
            .map_err(|_| {
                self.abort(ScopeAbortCause::DescriptorContract {
                    code: "parse-capability-reentered",
                })
            })?
            .poll();
        result.map_err(|cause| self.abort(ScopeAbortCause::Control(cause)))
    }

    fn reserve(
        &self,
        class: ImportStorageClass,
        bytes: u64,
    ) -> ParseResult<Option<ImportReservation>> {
        self.preflight_active()?;
        if bytes == 0 {
            return Ok(None);
        }
        let (phase, dimension) = class.accounting();
        self.coordinator
            .try_borrow_mut()
            .map_err(|_| {
                self.abort(ScopeAbortCause::DescriptorContract {
                    code: "parse-capability-reentered",
                })
            })?
            .checkpoint()
            .reserve(phase, dimension, bytes)
            .map(Some)
            .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause)))
    }

    fn reserve_inventory(
        &self,
        class: ImportStorageClass,
        bytes: u64,
    ) -> Result<ImportReservation, ScopeAbortCause> {
        let (phase, _) = class.accounting();
        self.coordinator
            .try_borrow_mut()
            .map_err(|_| ScopeAbortCause::DescriptorContract {
                code: "parse-capability-reentered",
            })?
            .checkpoint()
            .reserve(phase, ImportDimension::ScratchBytes, bytes)
            .map_err(ScopeAbortCause::Ledger)
    }

    fn resize_exact(
        &self,
        reservation: &mut Option<ImportReservation>,
        bytes: u64,
    ) -> ParseResult<()> {
        match (reservation.as_mut(), bytes) {
            (None, 0) => Ok(()),
            (Some(_), 0) => Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "metered-storage-unused-capacity-reservation",
            })),
            (Some(token), bytes) => self
                .coordinator
                .try_borrow_mut()
                .map_err(|_| {
                    self.abort(ScopeAbortCause::DescriptorContract {
                        code: "parse-capability-reentered",
                    })
                })?
                .checkpoint()
                .resize_reservation(token, bytes)
                .map_err(|cause| self.abort(ScopeAbortCause::Ledger(cause))),
            (None, _) => Err(self.abort(ScopeAbortCause::DescriptorContract {
                code: "metered-storage-capacity-without-reservation",
            })),
        }
    }

    fn capacity_error(&self, resource: &'static str, cause: CapacityPlanError) -> ParseFailure {
        match cause {
            CapacityPlanError::CapacityOverflow | CapacityPlanError::AllocationTooLarge { .. } => {
                self.abort(ScopeAbortCause::ResourceLimit {
                    resource,
                    limit: u64::MAX,
                })
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

    fn allocation_failure(&self, resource: &'static str) -> ParseFailure {
        self.abort(ScopeAbortCause::Allocation { resource })
    }

    fn qualification_failure(&self, resource: &'static str) -> ParseFailure {
        self.capacity_error(
            resource,
            CapacityPlanError::CapacityPlatformUnqualified {
                policy_revision: baozi_operation::CAPACITY_POLICY_REVISION,
                desired_capacity: 1,
                envelope_capacity: 1,
                actual_capacity: 2,
            },
        )
    }

    fn same_owner(&self, other: &Self) -> bool {
        std::ptr::eq(self.coordinator, other.coordinator)
            && std::ptr::eq(self.terminal, other.terminal)
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

        let desired = self
            .values
            .capacity()
            .saturating_mul(2)
            .max(required)
            .max(4);
        let plan = CapacityPlan::for_type::<T>(self.values.len(), self.values.capacity(), desired)
            .map_err(|cause| self.meter.capacity_error(self.resource, cause))?;
        let replacement_plan = CapacityPlan::for_type::<T>(0, 0, plan.envelope_capacity())
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

        let desired = self.value.capacity().saturating_mul(2).max(required).max(4);
        let plan =
            CapacityPlan::with_element_size(self.value.len(), self.value.capacity(), desired, 1)
                .map_err(|cause| self.meter.capacity_error(self.resource, cause))?;
        let replacement_plan = CapacityPlan::with_element_size(0, 0, plan.envelope_capacity(), 1)
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
        self.validate_append(value.len())?;
        append_metered_string_with_meter(&self.meter, &mut self.value, value)
    }

    /// Appends decoded bytes under a caller-owned bounded work chunk without polling again.
    pub fn try_push_str_reserved(&mut self, value: &str) -> ParseResult<()> {
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

impl<'scope, T> MeteredSceneVec<'scope, T> {
    pub fn try_push_parts(&mut self, mut parts: SceneValueParts<'scope, T>) -> ParseResult<()> {
        if !self.values.meter.same_owner(&parts.meter) {
            return Err(self
                .values
                .meter
                .abort(ScopeAbortCause::DescriptorContract {
                    code: "scene-collection-owner-mismatch",
                }));
        }
        self.values.try_reserve(1)?;
        let meter = self.values.meter;
        self.reservations
            .try_merge_preserving(
                &mut parts.reservations,
                "scene-collection-element-reservations",
                |bytes| meter.reserve_inventory(ImportStorageClass::SceneConstruction, bytes),
            )
            .map_err(|cause| meter.abort(cause))?;
        let value = parts
            .value
            .take()
            .expect("scene collection parts are consumed exactly once");
        self.values.values.push(value);
        Ok(())
    }

    pub fn try_into_scene_parts(self) -> ParseResult<SceneValueParts<'scope, Vec<T>>> {
        let Self {
            values,
            reservations,
        } = self;
        let mut reservations = reservations;
        // Keep element reservations outside the fallible conversion until the collection value
        // has an owner that will be dropped first on every early return.
        let mut parts = values.try_into_scene_parts()?;
        let meter = parts.meter;
        parts
            .reservations
            .try_merge_preserving(
                &mut reservations,
                "scene-collection-final-reservations",
                |bytes| meter.reserve_inventory(ImportStorageClass::SceneConstruction, bytes),
            )
            .map_err(|cause| meter.abort(cause))?;
        Ok(parts)
    }
}

/// Couples the metadata map with both the current and remaining entry owners during finalization.
/// Field order makes unwind drop the complete map before any unmerged entry reservation.
struct ScenePartsReservationFinalizer<'scope, T> {
    parts: SceneValueParts<'scope, T>,
    current: Option<SceneValueReservations>,
    entries: MeteredVec<'scope, SceneValueReservations>,
}

impl<'scope, T> ScenePartsReservationFinalizer<'scope, T> {
    fn try_finish(mut self) -> ParseResult<SceneValueParts<'scope, T>> {
        let meter = self.parts.meter;
        while let Some(reservations) = self.entries.pop() {
            debug_assert!(self.current.is_none());
            self.current = Some(reservations);
            #[cfg(test)]
            if fail_next_scene_parts_finalization_panic() {
                panic!("injected scene-parts finalization panic");
            }
            self.parts
                .reservations
                .try_merge_preserving(
                    self.current
                        .as_mut()
                        .expect("the current metadata entry remains coupled during merge"),
                    "metadata-map-final-reservations",
                    |bytes| meter.reserve_inventory(ImportStorageClass::SceneConstruction, bytes),
                )
                .map_err(|cause| meter.abort(cause))?;
            drop(self.current.take());
        }

        let Self {
            parts,
            current,
            entries,
        } = self;
        drop(current);
        drop(entries);
        Ok(parts)
    }
}

impl<'scope> MeteredMetadataMap<'scope> {
    pub fn try_insert_parts(
        &mut self,
        mut parts: SceneValueParts<'_, (String, MetadataValue)>,
    ) -> ParseResult<()> {
        let meter = self.values.meter;
        if !meter.same_owner(&parts.meter) {
            return Err(meter.abort(ScopeAbortCause::DescriptorContract {
                code: "metadata-map-owner-mismatch",
            }));
        }
        meter.poll()?;
        let (key, _) = parts
            .value
            .as_ref()
            .expect("metadata parts retain their entry before insertion");
        let position = self
            .values
            .value
            .iter()
            .position(|(candidate, _)| candidate >= key)
            .unwrap_or(self.values.len());
        let is_new = self.values.value.get(key).is_none();
        if self.entries.len() != self.values.len() {
            return Err(meter.abort(ScopeAbortCause::DescriptorContract {
                code: "metadata-map-entry-owner-mismatch",
            }));
        }
        self.values.try_reserve(usize::from(is_new))?;
        if is_new {
            self.entries.try_reserve(1)?;
        }
        let (key, value) = parts
            .value
            .take()
            .expect("metadata parts are consumed exactly once");
        let reservations = parts.reservations;
        let retired = self
            .values
            .value
            .insert_entry_reserved(key, value)
            .map_err(|_| {
                meter.abort(ScopeAbortCause::DescriptorContract {
                    code: "metadata-map-capacity-not-reserved",
                })
            })?;
        if is_new {
            self.entries.values.insert(position, reservations);
        } else {
            let old_reservations =
                std::mem::replace(&mut self.entries.values[position], reservations);
            if let Some(value) = retired {
                crate::metered_storage::RetiredPreparedMetadataEntry {
                    value,
                    reservations: old_reservations,
                }
                .drop_in_order();
            } else {
                drop(old_reservations);
            }
        }
        Ok(())
    }

    pub fn try_into_scene_parts(self) -> ParseResult<SceneValueParts<'scope, MetadataMap>> {
        let Self { values, entries } = self;
        ScenePartsReservationFinalizer {
            parts: values.try_into_scene_parts()?,
            current: None,
            entries,
        }
        .try_finish()
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
        _ => meter.abort(ScopeAbortCause::DescriptorContract {
            code: "metered-ordered-map-capacity-error-unknown",
        }),
    }
}

impl SceneValueReservations {
    fn try_merge_preserving(
        &mut self,
        other: &mut Self,
        resource: &'static str,
        reserve: impl FnMut(u64) -> Result<ImportReservation, ScopeAbortCause>,
    ) -> Result<(), ScopeAbortCause> {
        let payload_candidate =
            self.payload
                .prepare_replacement(other.payload.len(), resource, reserve)?;

        if let Some(candidate) = payload_candidate {
            candidate.commit();
        }

        append_accounted_reservations(&mut self.payload, &mut other.payload);
        drop(other.payload.take());
        Ok(())
    }
}

impl<'scope, T> MeteredVec<'scope, T> {
    pub fn try_into_scene_parts(mut self) -> ParseResult<SceneValueParts<'scope, Vec<T>>> {
        let mut reservations = prepare_scene_parts_reservations(
            &self.meter,
            self.class,
            self.capacity.as_ref(),
            self.resource,
        )?;
        let value = std::mem::take(&mut self.values);
        move_scene_part_capacity(&mut reservations, self.capacity.take());
        Ok(SceneValueParts {
            value: Some(value),
            reservations,
            meter: self.meter,
        })
    }
}

impl<'scope> MeteredString<'scope> {
    pub fn try_into_scene_parts(mut self) -> ParseResult<SceneValueParts<'scope, String>> {
        let mut reservations = prepare_scene_parts_reservations(
            &self.meter,
            self.class,
            self.capacity.as_ref(),
            self.resource,
        )?;
        let value = std::mem::take(&mut self.value);
        move_scene_part_capacity(&mut reservations, self.capacity.take());
        Ok(SceneValueParts {
            value: Some(value),
            reservations,
            meter: self.meter,
        })
    }
}

impl<'scope, K, V> MeteredOrderedMap<'scope, K, V> {
    pub fn try_into_scene_parts(
        mut self,
    ) -> ParseResult<SceneValueParts<'scope, OrderedMap<K, V>>> {
        let mut reservations = prepare_scene_parts_reservations(
            &self.meter,
            self.class,
            self.capacity.as_ref(),
            self.resource,
        )?;
        let value = std::mem::replace(&mut self.value, OrderedMap::new());
        move_scene_part_capacity(&mut reservations, self.capacity.take());
        Ok(SceneValueParts {
            value: Some(value),
            reservations,
            meter: self.meter,
        })
    }
}

impl<'scope, T> SceneValueParts<'scope, T> {
    pub fn zip<U>(
        mut self,
        mut other: SceneValueParts<'scope, U>,
    ) -> ParseResult<SceneValueParts<'scope, (T, U)>> {
        if !self.meter.same_owner(&other.meter) {
            return Err(self.meter.abort(ScopeAbortCause::DescriptorContract {
                code: "scene-value-parts-owner-mismatch",
            }));
        }
        self.meter.poll()?;
        let meter = self.meter;
        self.reservations
            .try_merge_preserving(
                &mut other.reservations,
                "scene-value-parts-reservations",
                |bytes| meter.reserve_inventory(ImportStorageClass::SceneConstruction, bytes),
            )
            .map_err(|cause| meter.abort(cause))?;
        let left = self
            .value
            .take()
            .expect("left scene value parts are combined exactly once");
        let right = other
            .value
            .take()
            .expect("right scene value parts are combined exactly once");
        Ok(SceneValueParts {
            value: Some((left, right)),
            reservations: self.reservations,
            meter,
        })
    }

    pub fn finish(mut self) -> ParseResult<PreparedSceneValue<T>>
    where
        T: SceneBuildValue,
    {
        self.meter.poll()?;
        let value = self
            .value
            .take()
            .expect("scene value parts are finished exactly once");
        let prepared = PreparedSceneValue {
            value: Some(value),
            reservations: self.reservations,
        };
        validate_prepared_scene_value_with_meter(&self.meter, prepared)
    }
}

impl<'scope> SceneValueParts<'scope, String> {
    pub fn into_some(self) -> SceneValueParts<'scope, Option<String>> {
        self.map_owned(Some)
    }

    pub fn discard_into_none(mut self) -> SceneValueParts<'scope, Option<String>> {
        let value = self
            .value
            .take()
            .expect("scene string parts are discarded exactly once");
        drop(value);
        drop(self.reservations);
        SceneValueParts {
            value: Some(None),
            reservations: SceneValueReservations::new(),
            meter: self.meter,
        }
    }

    pub fn into_external_texture_source(self) -> SceneValueParts<'scope, TextureSource> {
        self.map_owned(|uri| TextureSource::External { uri })
    }

    pub fn into_custom_vertex_attribute_semantic(
        self,
    ) -> SceneValueParts<'scope, VertexAttributeSemantic> {
        self.map_owned(VertexAttributeSemantic::Custom)
    }

    pub fn into_metadata_bool_entry(
        self,
        value: bool,
    ) -> SceneValueParts<'scope, (String, MetadataValue)> {
        self.map_owned(|key| (key, MetadataValue::Bool(value)))
    }

    pub fn into_metadata_i64_entry(
        self,
        value: i64,
    ) -> SceneValueParts<'scope, (String, MetadataValue)> {
        self.map_owned(|key| (key, MetadataValue::I64(value)))
    }

    pub fn into_metadata_f64_entry(
        self,
        value: f64,
    ) -> SceneValueParts<'scope, (String, MetadataValue)> {
        self.map_owned(|key| (key, MetadataValue::F64(value)))
    }
}

impl<'scope> SceneValueParts<'scope, (String, String)> {
    pub fn into_metadata_string_entry(self) -> SceneValueParts<'scope, (String, MetadataValue)> {
        self.map_owned(|(key, value)| (key, MetadataValue::String(value)))
    }
}

impl<'scope> SceneValueParts<'scope, Vec<f32>> {
    pub fn into_vertex_attribute_data(self) -> SceneValueParts<'scope, VertexAttributeData> {
        self.map_owned(VertexAttributeData::F32)
    }
}

impl<'scope> SceneValueParts<'scope, Vec<u32>> {
    pub fn into_vertex_attribute_data(self) -> SceneValueParts<'scope, VertexAttributeData> {
        self.map_owned(VertexAttributeData::U32)
    }
}

impl<'scope> SceneValueParts<'scope, Vec<i32>> {
    pub fn into_vertex_attribute_data(self) -> SceneValueParts<'scope, VertexAttributeData> {
        self.map_owned(VertexAttributeData::I32)
    }
}

impl<'scope> SceneValueParts<'scope, ((String, VertexAttributeSemantic), VertexAttributeData)> {
    pub fn into_vertex_attribute(self) -> SceneValueParts<'scope, VertexAttribute> {
        self.map_owned(|((name, semantic), data)| VertexAttribute {
            name,
            semantic,
            data,
            metadata: MetadataMap::new(),
        })
    }
}

impl<'scope> SceneValueParts<'scope, Option<String>> {
    pub fn into_mesh(self, config: MeshBuildConfig) -> SceneValueParts<'scope, Mesh> {
        self.map_owned(|name| Mesh {
            name,
            topology: config.topology,
            material: config.material,
            bounds: config.bounds,
            ..Mesh::default()
        })
    }

    pub fn into_material(self, config: MaterialBuildConfig) -> SceneValueParts<'scope, Material> {
        self.map_owned(|name| Material {
            name,
            shading_model: config.shading_model,
            base_color: config.base_color,
            metallic: config.metallic,
            roughness: config.roughness,
            emissive: config.emissive,
            alpha_mode: config.alpha_mode,
            alpha_cutoff: config.alpha_cutoff,
            double_sided: config.double_sided,
            ..Material::default()
        })
    }

    pub fn into_node(self, config: NodeBuildConfig) -> SceneValueParts<'scope, Node> {
        self.map_owned(|name| Node {
            name,
            transform: config.transform,
            camera: config.camera,
            light: config.light,
            ..Node::default()
        })
    }

    pub fn into_texture_slot(
        self,
        config: TextureSlotBuildConfig,
    ) -> SceneValueParts<'scope, TextureSlot> {
        self.map_owned(|source_key| TextureSlot {
            texture: config.texture,
            role: config.role,
            color_space: config.color_space,
            uv_set: config.uv_set,
            scale: config.scale,
            transform: config.transform,
            source_key,
        })
    }

    pub fn into_camera(self, config: CameraBuildConfig) -> SceneValueParts<'scope, Camera> {
        self.map_owned(|name| Camera {
            name,
            projection: config.projection,
            metadata: MetadataMap::new(),
        })
    }

    pub fn into_skin(self, config: SkinBuildConfig) -> SceneValueParts<'scope, Skin> {
        self.map_owned(|name| Skin {
            name,
            skeleton_root: config.skeleton_root,
            ..Skin::default()
        })
    }
}

impl<'scope> SceneValueParts<'scope, (Option<String>, TextureSource)> {
    pub fn into_texture(self, config: TextureBuildConfig) -> SceneValueParts<'scope, Texture> {
        self.map_owned(|(name, source)| Texture {
            name,
            source,
            sampler: config.sampler,
            metadata: MetadataMap::new(),
        })
    }
}

impl<'scope> SceneValueParts<'scope, Mesh> {
    pub fn with_positions(self, values: SceneValueParts<'scope, Vec<Vec3>>) -> ParseResult<Self> {
        Ok(self.zip(values)?.map_owned(|(mut mesh, values)| {
            mesh.positions = values;
            mesh
        }))
    }

    pub fn with_normals(self, values: SceneValueParts<'scope, Vec<Vec3>>) -> ParseResult<Self> {
        Ok(self.zip(values)?.map_owned(|(mut mesh, values)| {
            mesh.normals = values;
            mesh
        }))
    }

    pub fn with_tangents(self, values: SceneValueParts<'scope, Vec<Vec4>>) -> ParseResult<Self> {
        Ok(self.zip(values)?.map_owned(|(mut mesh, values)| {
            mesh.tangents = values;
            mesh
        }))
    }

    pub fn with_texcoords(
        self,
        values: SceneValueParts<'scope, Vec<Vec<Vec2>>>,
    ) -> ParseResult<Self> {
        Ok(self.zip(values)?.map_owned(|(mut mesh, values)| {
            mesh.texcoords = values;
            mesh
        }))
    }

    pub fn with_colors(
        self,
        values: SceneValueParts<'scope, Vec<Vec<Color>>>,
    ) -> ParseResult<Self> {
        Ok(self.zip(values)?.map_owned(|(mut mesh, values)| {
            mesh.colors = values;
            mesh
        }))
    }

    pub fn with_indices(self, values: SceneValueParts<'scope, Vec<u32>>) -> ParseResult<Self> {
        Ok(self.zip(values)?.map_owned(|(mut mesh, values)| {
            mesh.indices = values;
            mesh
        }))
    }

    pub fn with_face_vertex_counts(
        self,
        values: SceneValueParts<'scope, Vec<u32>>,
    ) -> ParseResult<Self> {
        Ok(self.zip(values)?.map_owned(|(mut mesh, values)| {
            mesh.face_vertex_counts = values;
            mesh
        }))
    }

    pub fn with_joint_indices(
        self,
        values: SceneValueParts<'scope, Vec<[u16; 4]>>,
    ) -> ParseResult<Self> {
        Ok(self.zip(values)?.map_owned(|(mut mesh, values)| {
            mesh.joint_indices = values;
            mesh
        }))
    }

    pub fn with_joint_weights(
        self,
        values: SceneValueParts<'scope, Vec<[f32; 4]>>,
    ) -> ParseResult<Self> {
        Ok(self.zip(values)?.map_owned(|(mut mesh, values)| {
            mesh.joint_weights = values;
            mesh
        }))
    }

    pub fn with_custom_attributes(
        self,
        values: SceneValueParts<'scope, Vec<VertexAttribute>>,
    ) -> ParseResult<Self> {
        Ok(self.zip(values)?.map_owned(|(mut mesh, values)| {
            mesh.custom_attributes = values;
            mesh
        }))
    }

    pub fn with_metadata(self, values: SceneValueParts<'scope, MetadataMap>) -> ParseResult<Self> {
        Ok(self.zip(values)?.map_owned(|(mut mesh, values)| {
            mesh.metadata = values;
            mesh
        }))
    }
}

impl<'scope> SceneValueParts<'scope, Material> {
    pub fn with_texture_slots(
        self,
        values: SceneValueParts<'scope, Vec<TextureSlot>>,
    ) -> ParseResult<Self> {
        Ok(self.zip(values)?.map_owned(|(mut material, values)| {
            material.texture_slots = values;
            material
        }))
    }

    pub fn with_metadata(self, values: SceneValueParts<'scope, MetadataMap>) -> ParseResult<Self> {
        Ok(self.zip(values)?.map_owned(|(mut material, values)| {
            material.metadata = values;
            material
        }))
    }
}

impl<'scope> SceneValueParts<'scope, Skin> {
    pub fn with_joints(self, values: SceneValueParts<'scope, Vec<NodeId>>) -> ParseResult<Self> {
        Ok(self.zip(values)?.map_owned(|(mut skin, values)| {
            skin.joints = values;
            skin
        }))
    }

    pub fn with_inverse_bind_matrices(
        self,
        values: SceneValueParts<'scope, Vec<Mat4>>,
    ) -> ParseResult<Self> {
        Ok(self.zip(values)?.map_owned(|(mut skin, values)| {
            skin.inverse_bind_matrices = values;
            skin
        }))
    }
}

fn prepare_scene_parts_reservations(
    meter: &StorageMeter<'_>,
    class: ImportStorageClass,
    capacity: Option<&ImportReservation>,
    resource: &'static str,
) -> ParseResult<SceneValueReservations> {
    if class != ImportStorageClass::SceneConstruction {
        return Err(meter.abort(ScopeAbortCause::DescriptorContract {
            code: "non-scene-storage-converted-to-scene-parts",
        }));
    }
    if let Some(capacity) = capacity {
        meter
            .coordinator
            .try_borrow()
            .map_err(|_| {
                meter.abort(ScopeAbortCause::DescriptorContract {
                    code: "parse-capability-reentered",
                })
            })?
            .validate_live_reservation(capacity)
            .map_err(|cause| meter.abort(ScopeAbortCause::Ledger(cause)))?;
    }
    let mut reservations = SceneValueReservations::new();
    if capacity.is_some() {
        reservations
            .payload
            .try_reserve(1, resource, |bytes| {
                meter.reserve_inventory(ImportStorageClass::SceneConstruction, bytes)
            })
            .map_err(|cause| meter.abort(cause))?;
    }
    Ok(reservations)
}

fn move_scene_part_capacity(
    reservations: &mut SceneValueReservations,
    capacity: Option<ImportReservation>,
) {
    if let Some(capacity) = capacity {
        reservations.payload.push_reserved_infallible(capacity);
    }
}

#[cfg(test)]
std::thread_local! {
    static FAIL_NEXT_METERED_QUALIFICATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static PANIC_NEXT_SCENE_PARTS_FINALIZATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn fail_next_metered_qualification() -> bool {
    FAIL_NEXT_METERED_QUALIFICATION.with(|fault| fault.replace(false))
}

#[cfg(not(test))]
const fn fail_next_metered_qualification() -> bool {
    false
}

#[cfg(test)]
fn fail_next_scene_parts_finalization_panic() -> bool {
    PANIC_NEXT_SCENE_PARTS_FINALIZATION.with(|fault| fault.replace(false))
}

fn append_accounted_reservations(
    target: &mut AccountedReservationVec,
    source: &mut AccountedReservationVec,
) {
    assert!(
        target.has_slots(source.len()),
        "reservation merge must be fully admitted before mutation"
    );
    target
        .try_append_accounted(source)
        .expect("an admitted reservation merge cannot fail");
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

fn validate_prepared_scene_value<T: SceneBuildValue>(
    scope: &ParseScope<'_>,
    prepared: PreparedSceneValue<T>,
) -> ParseResult<PreparedSceneValue<T>> {
    validate_prepared_scene_value_with_meter(&scope.storage_meter(), prepared)
}

fn validate_prepared_scene_value_with_meter<T: SceneBuildValue>(
    meter: &StorageMeter<'_>,
    prepared: PreparedSceneValue<T>,
) -> ParseResult<PreparedSceneValue<T>> {
    meter.poll()?;
    let footprint = prepared
        .value
        .as_ref()
        .expect("a new prepared scene value owns its value")
        .scene_value_footprint()
        .map_err(|cause| meter.abort(ScopeAbortCause::SceneAccounting(cause)))?;
    validate_scene_value_reservations_with_meter(meter, &prepared.reservations, footprint)?;
    Ok(prepared)
}

fn validate_scene_value_reservations(
    scope: &ParseScope<'_>,
    reservations: &SceneValueReservations,
    footprint: SceneValueFootprint,
) -> ParseResult<()> {
    validate_scene_value_reservations_with_meter(&scope.storage_meter(), reservations, footprint)
}

fn validate_scene_value_reservations_with_meter(
    meter: &StorageMeter<'_>,
    reservations: &SceneValueReservations,
    footprint: SceneValueFootprint,
) -> ParseResult<()> {
    let coordinator = meter.coordinator.try_borrow().map_err(|_| {
        meter.abort(ScopeAbortCause::DescriptorContract {
            code: "parse-capability-reentered",
        })
    })?;
    for reservation in reservations.payload.iter() {
        coordinator
            .validate_live_reservation(reservation)
            .map_err(|cause| meter.abort(ScopeAbortCause::Ledger(cause)))?;
    }
    drop(coordinator);
    let actual = reservation_bytes(&reservations.payload).ok_or_else(|| {
        meter.abort(ScopeAbortCause::DescriptorContract {
            code: "scene-value-reservation-bytes-overflow",
        })
    })?;
    if actual != footprint.payload_bytes() {
        return Err(meter.abort(ScopeAbortCause::DescriptorContract {
            code: "scene-value-payload-reservation-under-covered",
        }));
    }
    Ok(())
}

fn reservation_bytes(reservations: &AccountedReservationVec) -> Option<u64> {
    reservations.iter().try_fold(0u64, |total, reservation| {
        total.checked_add(reservation.bytes())
    })
}

struct ImportLiveReservation<'scope> {
    reservation: ImportReservation,
    scope: PhantomData<&'scope ()>,
}

impl ImportLiveReservation<'_> {
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

    let mut bytes = AccountedVec::new();
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
        bytes.try_reserve_capacity(capacity, "asset-snapshot", |capacity_bytes| {
            coordinator
                .borrow_mut()
                .checkpoint()
                .reserve(ledger_phase, dimension, capacity_bytes)
                .map_err(ScopeAbortCause::Ledger)
        })?;
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
            bytes.try_reserve(count, "asset-snapshot", |capacity_bytes| {
                coordinator
                    .borrow_mut()
                    .checkpoint()
                    .reserve(ledger_phase, dimension, capacity_bytes)
                    .map_err(ScopeAbortCause::Ledger)
            })?;
        }
        bytes
            .try_extend_from_slice_reserved(&chunk[..count])
            .map_err(|()| ScopeAbortCause::DescriptorContract {
                code: "asset-snapshot-slots-not-admitted",
            })?;
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

    Ok(bytes)
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

#[derive(Default)]
enum SceneCapacityReservation {
    #[default]
    Empty,
    Live(ImportReservation),
    Replacing {
        old: Option<ImportReservation>,
        new: ImportReservation,
    },
}

impl SceneCapacityReservation {
    fn is_stable(&self) -> bool {
        !matches!(self, Self::Replacing { .. })
    }

    fn accumulate(&self, snapshot: &mut SceneReservationSnapshot) -> Result<(), &'static str> {
        match self {
            Self::Empty => Ok(()),
            Self::Live(reservation) => snapshot.add(reservation),
            Self::Replacing { .. } => Err("scene-reservation-replacement-in-progress"),
        }
    }

    fn try_begin_replacement(
        &mut self,
        new: ImportReservation,
    ) -> Result<SceneCapacityReplacement<'_>, ImportReservation> {
        if matches!(self, Self::Replacing { .. }) {
            return Err(new);
        }
        let old = match std::mem::take(self) {
            Self::Empty => None,
            Self::Live(old) => Some(old),
            Self::Replacing { .. } => unreachable!("replacement state was checked before take"),
        };
        *self = Self::Replacing { old, new };
        Ok(SceneCapacityReplacement { slot: self })
    }

    fn take_live(&mut self) -> Result<Option<ImportReservation>, ()> {
        match std::mem::take(self) {
            Self::Empty => Ok(None),
            Self::Live(reservation) => Ok(Some(reservation)),
            replacing @ Self::Replacing { .. } => {
                *self = replacing;
                Err(())
            }
        }
    }

    fn restore_live(&mut self, reservation: ImportReservation) {
        debug_assert!(matches!(self, Self::Empty));
        *self = Self::Live(reservation);
    }
}

struct SceneCapacityReplacement<'slot> {
    slot: &'slot mut SceneCapacityReservation,
}

impl SceneCapacityReplacement<'_> {
    fn publish(self) -> Option<ImportReservation> {
        let SceneCapacityReservation::Replacing { old, new } = std::mem::take(self.slot) else {
            unreachable!("a replacement guard exclusively owns an armed slot");
        };
        *self.slot = SceneCapacityReservation::Live(new);
        old
    }
}

#[derive(Default)]
struct SceneCollectionReservationSlots {
    nodes: SceneCapacityReservation,
    meshes: SceneCapacityReservation,
    materials: SceneCapacityReservation,
    textures: SceneCapacityReservation,
    animations: SceneCapacityReservation,
    cameras: SceneCapacityReservation,
    lights: SceneCapacityReservation,
    skins: SceneCapacityReservation,
}

impl SceneCollectionReservationSlots {
    fn get_mut(&mut self, collection: SceneCollection) -> &mut SceneCapacityReservation {
        match collection {
            SceneCollection::Nodes => &mut self.nodes,
            SceneCollection::Meshes => &mut self.meshes,
            SceneCollection::Materials => &mut self.materials,
            SceneCollection::Textures => &mut self.textures,
            SceneCollection::Animations => &mut self.animations,
            SceneCollection::Cameras => &mut self.cameras,
            SceneCollection::Lights => &mut self.lights,
            SceneCollection::Skins => &mut self.skins,
        }
    }

    fn slots(&self) -> [&SceneCapacityReservation; 8] {
        [
            &self.nodes,
            &self.meshes,
            &self.materials,
            &self.textures,
            &self.animations,
            &self.cameras,
            &self.lights,
            &self.skins,
        ]
    }

    fn slots_mut(&mut self) -> [&mut SceneCapacityReservation; 8] {
        [
            &mut self.nodes,
            &mut self.meshes,
            &mut self.materials,
            &mut self.textures,
            &mut self.animations,
            &mut self.cameras,
            &mut self.lights,
            &mut self.skins,
        ]
    }
}

#[derive(Default)]
struct NodeReservationSlots {
    children: SceneCapacityReservation,
    mesh_bindings: SceneCapacityReservation,
}

impl NodeReservationSlots {
    fn slots(&self) -> [&SceneCapacityReservation; 2] {
        [&self.children, &self.mesh_bindings]
    }

    fn slots_mut(&mut self) -> [&mut SceneCapacityReservation; 2] {
        [&mut self.children, &mut self.mesh_bindings]
    }
}

#[derive(Default)]
struct SceneReservationSnapshot {
    token_count: usize,
    bytes: u64,
}

impl SceneReservationSnapshot {
    fn add(&mut self, reservation: &ImportReservation) -> Result<(), &'static str> {
        self.token_count = self
            .token_count
            .checked_add(1)
            .ok_or("scene-reservation-count-overflow")?;
        self.bytes = self
            .bytes
            .checked_add(reservation.bytes())
            .ok_or("draft-reservation-bytes-overflow")?;
        Ok(())
    }
}

struct SceneReservationInventory {
    inline: SceneCapacityReservation,
    collections: SceneCollectionReservationSlots,
    nodes: AccountedVec<NodeReservationSlots>,
    metadata: SceneCapacityReservation,
    metadata_entries: AccountedVec<SceneValueReservations>,
    payload: AccountedReservationVec,
}

impl SceneReservationInventory {
    fn new(
        inline: ImportReservation,
        root_nodes: ImportReservation,
        nodes: AccountedVec<NodeReservationSlots>,
    ) -> Self {
        Self {
            inline: SceneCapacityReservation::Live(inline),
            collections: SceneCollectionReservationSlots {
                nodes: SceneCapacityReservation::Live(root_nodes),
                ..SceneCollectionReservationSlots::default()
            },
            nodes,
            metadata: SceneCapacityReservation::Empty,
            metadata_entries: AccountedVec::new(),
            payload: AccountedReservationVec::new(),
        }
    }

    fn snapshot(&self) -> Result<SceneReservationSnapshot, &'static str> {
        let mut snapshot = SceneReservationSnapshot::default();
        self.inline.accumulate(&mut snapshot)?;
        for slot in self.collections.slots() {
            slot.accumulate(&mut snapshot)?;
        }
        for node in self.nodes.iter() {
            for slot in node.slots() {
                slot.accumulate(&mut snapshot)?;
            }
        }
        self.metadata.accumulate(&mut snapshot)?;
        for entry in self.metadata_entries.iter() {
            for reservation in entry.payload.iter() {
                snapshot.add(reservation)?;
            }
        }
        for reservation in self.payload.iter() {
            snapshot.add(reservation)?;
        }
        Ok(snapshot)
    }

    fn typed_token_count(&self) -> Result<usize, &'static str> {
        self.snapshot()?
            .token_count
            .checked_sub(self.payload.len())
            .ok_or("scene-reservation-count-underflow")
    }

    fn transfer_into(&mut self, target: &mut AccountedReservationVec) -> Result<(), ()> {
        let typed_count = self.typed_token_count().map_err(|_| ())?;
        if !self.payload.has_slots(typed_count) {
            return Err(());
        }
        let total_count = self.payload.len().checked_add(typed_count).ok_or(())?;
        if !target.has_slots(total_count) {
            return Err(());
        }

        transfer_scene_slot(&mut self.inline, &mut self.payload)?;
        for slot in self.collections.slots_mut() {
            transfer_scene_slot(slot, &mut self.payload)?;
        }
        for node in self.nodes.iter_mut() {
            for slot in node.slots_mut() {
                transfer_scene_slot(slot, &mut self.payload)?;
            }
        }
        transfer_scene_slot(&mut self.metadata, &mut self.payload)?;
        for entry in self.metadata_entries.iter_mut() {
            self.payload.try_append_accounted(&mut entry.payload)?;
        }
        target.try_append_accounted(&mut self.payload)
    }
}

fn transfer_scene_slot(
    slot: &mut SceneCapacityReservation,
    target: &mut AccountedReservationVec,
) -> Result<(), ()> {
    let Some(reservation) = slot.take_live()? else {
        return Ok(());
    };
    match target.try_push_reserved(reservation) {
        Ok(()) => Ok(()),
        Err(reservation) => {
            slot.restore_live(reservation);
            Err(())
        }
    }
}

struct AccountedSceneStorage {
    builder: SceneBuilder,
    inventory: SceneReservationInventory,
}

fn reserve_scene_inventory_capacity(
    scope: &ParseScope<'_>,
    bytes: u64,
) -> Result<ImportReservation, ScopeAbortCause> {
    scope
        .coordinator
        .try_borrow_mut()
        .map_err(|_| ScopeAbortCause::DescriptorContract {
            code: "parse-capability-reentered",
        })?
        .checkpoint()
        .reserve(
            ImportLedgerPhase::SceneConstruction,
            ImportDimension::ScratchBytes,
            bytes,
        )
        .map_err(ScopeAbortCause::Ledger)
}

fn reserve_scene_candidate<'scope>(
    scope: &'scope ParseScope<'_>,
    admitted: u64,
) -> ParseResult<Option<ImportLiveReservation<'scope>>> {
    if admitted == 0 {
        return Ok(None);
    }
    scope
        .reserve_live(ImportStorageClass::SceneConstruction, admitted)
        .map(Some)
}

fn qualify_scene_candidate(
    scope: &ParseScope<'_>,
    token: &mut Option<ImportLiveReservation<'_>>,
    admitted: u64,
    actual: u64,
) -> ParseResult<()> {
    if actual > admitted {
        return Err(scope.abort(ScopeAbortCause::DescriptorContract {
            code: "scene-builder-capacity-reservation-underestimated",
        }));
    }
    match (token.as_mut(), actual) {
        (None, 0) => Ok(()),
        (Some(_), 0) => Err(scope.abort(ScopeAbortCause::DescriptorContract {
            code: "scene-builder-unused-capacity-reservation",
        })),
        (Some(token), bytes) => scope.resize_live(token, bytes),
        (None, _) => Err(scope.abort(ScopeAbortCause::DescriptorContract {
            code: "scene-builder-capacity-growth-without-reservation",
        })),
    }
}

fn metadata_error(scope: &ParseScope<'_>, cause: OrderedMapError) -> ParseFailure {
    match cause {
        OrderedMapError::CapacityOverflow => {
            scope.resource_limit_exceeded("scene-metadata-capacity", u64::MAX)
        }
        OrderedMapError::AllocationFailed => scope.allocation_failure("scene-metadata-capacity"),
        OrderedMapError::CapacityNotReserved => scope.abort(ScopeAbortCause::DescriptorContract {
            code: "scene-metadata-capacity-not-reserved",
        }),
        _ => scope.abort(ScopeAbortCause::DescriptorContract {
            code: "scene-metadata-capacity-error-unknown",
        }),
    }
}

struct AccountedSceneCandidate<'scope, Candidate> {
    candidate: Candidate,
    token: Option<ImportLiveReservation<'scope>>,
}

struct AccountedPreparedSceneCandidate<'scope, Candidate> {
    candidate: Candidate,
    reservations: SceneValueReservations,
    token: Option<ImportLiveReservation<'scope>>,
}

struct AccountedPreparedNodeCandidate<'scope, Candidate> {
    candidate: Candidate,
    reservations: SceneValueReservations,
    nodes_token: Option<ImportLiveReservation<'scope>>,
    children_token: Option<ImportLiveReservation<'scope>>,
}

type SceneInsertPreparer<T, Id> =
    for<'owner> fn(
        &'owner mut SceneBuilder,
        T,
        SceneCapacityPlan,
    ) -> Result<PreparedSceneInsert<'owner, T, Id>, SceneBuildError>;

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
    storage: AccountedSceneStorage,
}

impl ScopedSceneBuilder<'_, '_> {
    pub fn root(&self) -> NodeId {
        self.storage.builder.root()
    }

    pub fn mesh_count(&self) -> usize {
        self.storage.builder.mesh_count()
    }

    pub fn add_prepared_node(&mut self, node: PreparedSceneValue<Node>) -> ParseResult<NodeId> {
        self.add_prepared_child_node(self.storage.builder.root(), node)
    }

    pub fn add_prepared_child_node(
        &mut self,
        parent: NodeId,
        node: PreparedSceneValue<Node>,
    ) -> ParseResult<NodeId> {
        self.add_prepared_node_accounted(parent, node)
    }

    pub fn add_prepared_mesh(&mut self, mesh: PreparedSceneValue<Mesh>) -> ParseResult<MeshId> {
        self.add_prepared_collection_accounted(
            SceneCollection::Meshes,
            mesh,
            SceneBuilder::prepare_add_mesh,
        )
    }

    pub fn add_prepared_material(
        &mut self,
        material: PreparedSceneValue<Material>,
    ) -> ParseResult<MaterialId> {
        self.add_prepared_collection_accounted(
            SceneCollection::Materials,
            material,
            SceneBuilder::prepare_add_material,
        )
    }

    pub fn add_prepared_texture(
        &mut self,
        texture: PreparedSceneValue<Texture>,
    ) -> ParseResult<TextureId> {
        self.add_prepared_collection_accounted(
            SceneCollection::Textures,
            texture,
            SceneBuilder::prepare_add_texture,
        )
    }

    pub fn add_prepared_animation(
        &mut self,
        animation: PreparedSceneValue<Animation>,
    ) -> ParseResult<AnimationId> {
        self.add_prepared_collection_accounted(
            SceneCollection::Animations,
            animation,
            SceneBuilder::prepare_add_animation,
        )
    }

    pub fn add_prepared_camera(
        &mut self,
        camera: PreparedSceneValue<Camera>,
    ) -> ParseResult<CameraId> {
        self.add_prepared_collection_accounted(
            SceneCollection::Cameras,
            camera,
            SceneBuilder::prepare_add_camera,
        )
    }

    pub fn add_prepared_light(&mut self, light: PreparedSceneValue<Light>) -> ParseResult<LightId> {
        self.add_prepared_collection_accounted(
            SceneCollection::Lights,
            light,
            SceneBuilder::prepare_add_light,
        )
    }

    pub fn add_prepared_skin(&mut self, skin: PreparedSceneValue<Skin>) -> ParseResult<SkinId> {
        self.add_prepared_collection_accounted(
            SceneCollection::Skins,
            skin,
            SceneBuilder::prepare_add_skin,
        )
    }

    pub fn node_mesh_binding_count(&mut self, id: NodeId) -> Option<usize> {
        if self.scope.preflight_active().is_err() {
            return None;
        }
        self.storage
            .builder
            .node_mut(id)
            .map(|node| node.mesh_bindings.len())
    }

    pub fn add_node_mesh_binding(&mut self, id: NodeId, binding: MeshBinding) -> ParseResult<()> {
        let scope = self.scope;
        scope.poll_active()?;
        let storage = &mut self.storage;
        let node_slot = storage.inventory.nodes.get(id.index()).ok_or_else(|| {
            scope.abort(ScopeAbortCause::DescriptorContract {
                code: "node-mesh-binding-node-out-of-range",
            })
        })?;
        if !node_slot.mesh_bindings.is_stable() {
            return Err(scope.abort(ScopeAbortCause::DescriptorContract {
                code: "scene-reservation-replacement-in-progress",
            }));
        }
        let plan = storage
            .builder
            .node_mesh_binding_capacity_plan(id, 1)
            .map_err(|cause| scope.abort(ScopeAbortCause::SceneBuild(cause)))?;
        let admitted = plan
            .envelope_delta_bytes()
            .map_err(|cause| scope.abort(ScopeAbortCause::SceneBuild(cause)))?;
        let token = reserve_scene_candidate(scope, admitted)?;
        let candidate = storage
            .builder
            .prepare_add_node_mesh_binding(id, binding, plan)
            .map_err(|cause| scope.abort(ScopeAbortCause::SceneBuild(cause)))?;
        let mut accounted = AccountedSceneCandidate { candidate, token };
        let actual = accounted.candidate.replacement_bytes();
        qualify_scene_candidate(scope, &mut accounted.token, admitted, actual)?;
        let replaces = accounted.candidate.replaces_allocation();
        if replaces != accounted.token.is_some() {
            return Err(scope.abort(ScopeAbortCause::DescriptorContract {
                code: "scene-builder-replacement-token-mismatch",
            }));
        }
        scope.poll_active()?;

        if !replaces {
            accounted.candidate.commit();
            return Ok(());
        }
        let new = accounted
            .token
            .take()
            .expect("a replacement candidate has a qualified reservation")
            .into_reservation();
        let slot = &mut storage.inventory.nodes[id.index()].mesh_bindings;
        let guard = match slot.try_begin_replacement(new) {
            Ok(guard) => guard,
            Err(new) => {
                drop(accounted.candidate);
                drop(new);
                return Err(scope.abort(ScopeAbortCause::DescriptorContract {
                    code: "scene-reservation-replacement-in-progress",
                }));
            }
        };
        accounted.candidate.commit();
        drop(guard.publish());
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
            .storage
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
        *self.storage.builder.space_mut() = space;
        Ok(())
    }

    pub fn insert_prepared_metadata(
        &mut self,
        mut prepared: PreparedSceneValue<(String, MetadataValue)>,
    ) -> ParseResult<()> {
        let scope = self.scope;
        scope.poll_active()?;
        let footprint = prepared
            .value
            .as_ref()
            .expect("a prepared metadata entry is inserted exactly once")
            .scene_value_footprint()
            .map_err(|cause| scope.abort(ScopeAbortCause::SceneAccounting(cause)))?;
        validate_scene_value_reservations(scope, &prepared.reservations, footprint)?;
        let storage = &mut self.storage;
        let (is_new, position, metadata_len) = {
            let key = &prepared
                .value
                .as_ref()
                .expect("a prepared metadata entry retains its key before insertion")
                .0;
            let metadata = storage.builder.metadata_mut();
            let position = metadata
                .keys()
                .position(|candidate| candidate.as_str() >= key.as_str())
                .unwrap_or(metadata.len());
            (
                !metadata.contains_key(key.as_str()),
                position,
                metadata.len(),
            )
        };
        if storage.inventory.metadata_entries.len() != metadata_len
            || !storage.inventory.metadata.is_stable()
        {
            return Err(scope.abort(ScopeAbortCause::DescriptorContract {
                code: "scene-metadata-reservation-inventory-mismatch",
            }));
        }
        if is_new {
            storage
                .inventory
                .metadata_entries
                .try_reserve(1, "scene-metadata-reservation-slots", |bytes| {
                    reserve_scene_inventory_capacity(scope, bytes)
                })
                .map_err(|cause| scope.abort(cause))?;
        }
        let key = &prepared
            .value
            .as_ref()
            .expect("a prepared metadata entry retains its key during admission")
            .0;
        let plan = storage
            .builder
            .metadata_capacity_plan(key)
            .map_err(|cause| metadata_error(scope, cause))?;
        let admitted = plan
            .envelope_bytes()
            .map_err(|cause| metadata_error(scope, cause))?;
        let token = reserve_scene_candidate(scope, admitted)?;
        let (key, value) = prepared.take_value();
        let candidate = storage
            .builder
            .prepare_insert_metadata(key, value, plan)
            .map_err(|cause| metadata_error(scope, cause))?;
        let mut accounted = AccountedPreparedSceneCandidate {
            candidate,
            reservations: prepared.take_reservations(),
            token,
        };
        let actual = accounted.candidate.replacement_bytes();
        qualify_scene_candidate(scope, &mut accounted.token, admitted, actual)?;
        let replaces = accounted.candidate.replaces_allocation();
        if replaces != accounted.token.is_some() {
            return Err(scope.abort(ScopeAbortCause::DescriptorContract {
                code: "scene-builder-replacement-token-mismatch",
            }));
        }
        scope.poll_active()?;
        if is_new
            && storage
                .inventory
                .metadata_entries
                .try_insert_reserved(position, SceneValueReservations::new())
                .is_err()
        {
            return Err(scope.abort(ScopeAbortCause::DescriptorContract {
                code: "scene-metadata-reservation-slot-not-admitted",
            }));
        }

        let guard = if replaces {
            let new = accounted
                .token
                .take()
                .expect("a metadata replacement has a qualified reservation")
                .into_reservation();
            match storage.inventory.metadata.try_begin_replacement(new) {
                Ok(guard) => Some(guard),
                Err(new) => {
                    drop(accounted.candidate);
                    drop(new);
                    return Err(scope.abort(ScopeAbortCause::DescriptorContract {
                        code: "scene-reservation-replacement-in-progress",
                    }));
                }
            }
        } else {
            None
        };
        let retired = accounted.candidate.commit();
        let retired_reservations = std::mem::replace(
            &mut storage.inventory.metadata_entries[position],
            accounted.reservations,
        );
        if let Some(value) = retired {
            crate::metered_storage::RetiredPreparedMetadataEntry {
                value,
                reservations: retired_reservations,
            }
            .drop_in_order();
        } else {
            drop(retired_reservations);
        }
        let retired_metadata_capacity = guard.and_then(SceneCapacityReplacement::publish);
        drop(retired_metadata_capacity);
        Ok(())
    }

    fn add_prepared_collection_accounted<T, Id>(
        &mut self,
        collection: SceneCollection,
        mut prepared: PreparedSceneValue<T>,
        prepare: SceneInsertPreparer<T, Id>,
    ) -> ParseResult<Id>
    where
        T: SceneBuildValue,
    {
        let scope = self.scope;
        scope.poll_active()?;
        let footprint = prepared
            .value
            .as_ref()
            .expect("a prepared scene value is inserted exactly once")
            .scene_value_footprint()
            .map_err(|cause| scope.abort(ScopeAbortCause::SceneAccounting(cause)))?;
        validate_scene_value_reservations(scope, &prepared.reservations, footprint)?;
        let storage = &mut self.storage;
        if !storage
            .inventory
            .collections
            .get_mut(collection)
            .is_stable()
        {
            return Err(scope.abort(ScopeAbortCause::DescriptorContract {
                code: "scene-reservation-replacement-in-progress",
            }));
        }
        let payload_count = prepared.reservations.payload.len();
        storage
            .inventory
            .payload
            .try_reserve(
                payload_count,
                "scene-builder-prepared-payload-reservations",
                |bytes| reserve_scene_inventory_capacity(scope, bytes),
            )
            .map_err(|cause| scope.abort(cause))?;
        let plan = storage
            .builder
            .collection_capacity_plan(collection, 1)
            .map_err(|cause| scope.abort(ScopeAbortCause::SceneBuild(cause)))?;
        let admitted = plan
            .envelope_delta_bytes()
            .map_err(|cause| scope.abort(ScopeAbortCause::SceneBuild(cause)))?;
        let token = reserve_scene_candidate(scope, admitted)?;
        let value = prepared.take_value();
        let candidate = prepare(&mut storage.builder, value, plan)
            .map_err(|cause| scope.abort(ScopeAbortCause::SceneBuild(cause)))?;
        let mut accounted = AccountedPreparedSceneCandidate {
            candidate,
            reservations: prepared.take_reservations(),
            token,
        };
        let actual = accounted.candidate.replacement_bytes();
        qualify_scene_candidate(scope, &mut accounted.token, admitted, actual)?;
        let replaces = accounted.candidate.replaces_allocation();
        if replaces != accounted.token.is_some() {
            return Err(scope.abort(ScopeAbortCause::DescriptorContract {
                code: "scene-builder-replacement-token-mismatch",
            }));
        }
        scope.poll_active()?;

        let replacement = if replaces {
            let new = accounted
                .token
                .take()
                .expect("a prepared collection replacement has a qualified reservation")
                .into_reservation();
            Some(
                storage
                    .inventory
                    .collections
                    .get_mut(collection)
                    .try_begin_replacement(new)
                    .expect("the prepared collection slot was prevalidated as stable"),
            )
        } else {
            None
        };
        storage
            .inventory
            .payload
            .try_append_accounted(&mut accounted.reservations.payload)
            .expect("prepared payload reservation slots were admitted before commit");
        let id = accounted.candidate.commit();
        let retired = replacement.and_then(SceneCapacityReplacement::publish);
        drop(retired);
        Ok(id)
    }

    fn add_prepared_node_accounted(
        &mut self,
        parent: NodeId,
        mut prepared: PreparedSceneValue<Node>,
    ) -> ParseResult<NodeId> {
        let scope = self.scope;
        scope.poll_active()?;
        let node = prepared
            .value
            .as_ref()
            .expect("a prepared node is inserted exactly once");
        if node.parent.is_some() || !node.children.is_empty() || !node.mesh_bindings.is_empty() {
            return Err(scope.abort(ScopeAbortCause::DescriptorContract {
                code: "prepared-node-must-not-preown-relationships",
            }));
        }
        let footprint = node
            .scene_value_footprint()
            .map_err(|cause| scope.abort(ScopeAbortCause::SceneAccounting(cause)))?;
        validate_scene_value_reservations(scope, &prepared.reservations, footprint)?;
        let storage = &mut self.storage;
        if !storage.inventory.collections.nodes.is_stable()
            || !storage
                .inventory
                .nodes
                .get(parent.index())
                .is_some_and(|slots| slots.children.is_stable())
            || storage.inventory.nodes.len() != storage.builder.node_count()
        {
            return Err(scope.abort(ScopeAbortCause::DescriptorContract {
                code: "scene-node-reservation-inventory-mismatch",
            }));
        }
        let payload_count = prepared.reservations.payload.len();
        storage
            .inventory
            .payload
            .try_reserve(
                payload_count,
                "scene-builder-prepared-node-payload-reservations",
                |bytes| reserve_scene_inventory_capacity(scope, bytes),
            )
            .map_err(|cause| scope.abort(cause))?;
        let plan = storage
            .builder
            .node_insert_capacity_plan(parent)
            .map_err(|cause| scope.abort(ScopeAbortCause::SceneBuild(cause)))?;
        let nodes_admitted = plan
            .node_envelope_bytes()
            .map_err(|cause| scope.abort(ScopeAbortCause::SceneBuild(cause)))?;
        let children_admitted = plan
            .parent_children_envelope_bytes()
            .map_err(|cause| scope.abort(ScopeAbortCause::SceneBuild(cause)))?;
        storage
            .inventory
            .nodes
            .try_reserve(1, "scene-node-reservation-slots", |bytes| {
                reserve_scene_inventory_capacity(scope, bytes)
            })
            .map_err(|cause| scope.abort(cause))?;
        let nodes_token = reserve_scene_candidate(scope, nodes_admitted)?;
        let children_token = reserve_scene_candidate(scope, children_admitted)?;
        let node = prepared.take_value();
        let candidate = storage
            .builder
            .prepare_add_child_node(parent, node, plan)
            .map_err(|cause| scope.abort(ScopeAbortCause::SceneBuild(cause)))?;
        let mut accounted = AccountedPreparedNodeCandidate {
            candidate,
            reservations: prepared.take_reservations(),
            nodes_token,
            children_token,
        };
        let nodes_actual = accounted.candidate.node_replacement_bytes();
        let children_actual = accounted.candidate.parent_children_replacement_bytes();
        qualify_scene_candidate(
            scope,
            &mut accounted.nodes_token,
            nodes_admitted,
            nodes_actual,
        )?;
        qualify_scene_candidate(
            scope,
            &mut accounted.children_token,
            children_admitted,
            children_actual,
        )?;
        let replaces_nodes = accounted.candidate.replaces_nodes();
        let replaces_children = accounted.candidate.replaces_parent_children();
        if replaces_nodes != accounted.nodes_token.is_some()
            || replaces_children != accounted.children_token.is_some()
        {
            return Err(scope.abort(ScopeAbortCause::DescriptorContract {
                code: "scene-builder-replacement-token-mismatch",
            }));
        }
        scope.poll_active()?;

        storage
            .inventory
            .nodes
            .push_reserved_infallible(NodeReservationSlots::default());
        let nodes_replacement = if replaces_nodes {
            let token = accounted
                .nodes_token
                .take()
                .expect("a prepared node replacement has a qualified reservation")
                .into_reservation();
            Some(
                storage
                    .inventory
                    .collections
                    .nodes
                    .try_begin_replacement(token)
                    .expect("the prepared node collection slot was prevalidated as stable"),
            )
        } else {
            None
        };
        let children_replacement = if replaces_children {
            let token = accounted
                .children_token
                .take()
                .expect("a prepared parent-child replacement has a qualified reservation")
                .into_reservation();
            Some(
                storage.inventory.nodes[parent.index()]
                    .children
                    .try_begin_replacement(token)
                    .expect("the prepared parent-child slot was prevalidated as stable"),
            )
        } else {
            None
        };
        storage
            .inventory
            .payload
            .try_append_accounted(&mut accounted.reservations.payload)
            .expect("prepared node payload slots were admitted before commit");
        let id = accounted.candidate.commit();
        let retired_nodes = nodes_replacement.and_then(SceneCapacityReplacement::publish);
        let retired_children = children_replacement.and_then(SceneCapacityReplacement::publish);
        drop(retired_nodes);
        drop(retired_children);
        Ok(id)
    }

    pub fn finish_draft(mut self) -> ParseResult<SceneDraft> {
        self.scope.poll_active()?;
        let footprint = self
            .storage
            .builder
            .storage_footprint()
            .map_err(|cause| self.scope.abort(ScopeAbortCause::SceneAccounting(cause)))?
            .accounted_payload_bytes();
        self.scope
            .seal_draft_inventory(&mut self.storage.inventory, footprint)?;
        Ok(self.storage.builder.finish_draft())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{ImportOperationControl, import_ledger_limits};
    use crate::{
        CapabilityStatus, DetectionOutcome, DetectionResult, DetectionScope, FormatCapability,
        FormatDescriptor, FormatEncoding, FormatImporter, ImportRequest, OptionCompileLimits,
        ReadConfidence, ReadHint, RegistryBuildLimits, RegistryBuilder,
    };
    use baozi_core::{FormatId, SceneDraft, Vec2, Vec3};
    use baozi_io::AssetPath;
    use baozi_io::{AssetLength, AuthorityDomain};
    use std::io::Cursor;

    #[derive(Clone, Copy)]
    enum QualificationContainer {
        Vec,
        String,
        OrderedMap,
    }

    struct QualificationFailureImporter {
        container: QualificationContainer,
    }

    struct ScenePartsImporter;

    struct PreparedMetadataImporter;

    #[derive(Clone, Copy)]
    enum PreparedNodeCase {
        Success,
        PreownedParent,
        PreownedChild,
        PreownedMeshBinding,
    }

    struct PreparedNodeImporter {
        case: PreparedNodeCase,
    }

    impl FormatImporter for QualificationFailureImporter {
        fn descriptor(&self) -> FormatDescriptor {
            FormatDescriptor::builder(
                FormatId::new("dev.baozi.test.metered-qualification")
                    .expect("valid test format id"),
                "Metered Qualification Test",
            )
            .extensions(["metered-qualification"])
            .encoding(FormatEncoding::Binary)
            .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
            .build()
            .expect("valid test descriptor")
        }

        fn detect(
            &self,
            _probe: &crate::Probe<'_>,
            _hint: &ReadHint,
            _scope: &DetectionScope<'_>,
        ) -> DetectionResult<DetectionOutcome> {
            Ok(DetectionOutcome::Match(ReadConfidence::Certain))
        }

        fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
            let failure = match self.container {
                QualificationContainer::Vec => {
                    let mut values = scope.metered_vec(
                        ImportStorageClass::FormatNormalization,
                        "owned-metered-test-vec",
                    );
                    values.try_push(7_u32)?;
                    let capacity = values.capacity();
                    let token_bytes = values.capacity.as_ref().map(ImportReservation::bytes);
                    FAIL_NEXT_METERED_QUALIFICATION.with(|fault| fault.set(true));
                    let result = values.try_reserve(capacity);
                    assert_eq!(values.as_slice(), [7]);
                    assert_eq!(values.capacity(), capacity);
                    assert_eq!(
                        values.capacity.as_ref().map(ImportReservation::bytes),
                        token_bytes
                    );
                    parse_failure(result)
                }
                QualificationContainer::String => {
                    let mut value = scope.metered_string(
                        ImportStorageClass::FormatNormalization,
                        "owned-metered-test-string",
                    );
                    value.try_push_str("x")?;
                    let capacity = value.capacity();
                    let token_bytes = value.capacity.as_ref().map(ImportReservation::bytes);
                    FAIL_NEXT_METERED_QUALIFICATION.with(|fault| fault.set(true));
                    let result = value.try_reserve(capacity);
                    assert_eq!(value.as_str(), "x");
                    assert_eq!(value.capacity(), capacity);
                    assert_eq!(
                        value.capacity.as_ref().map(ImportReservation::bytes),
                        token_bytes
                    );
                    parse_failure(result)
                }
                QualificationContainer::OrderedMap => {
                    let mut values = scope.metered_ordered_map(
                        ImportStorageClass::FormatNormalization,
                        "owned-metered-test-ordered-map",
                    );
                    values.try_insert(3_u32, 11_u32)?;
                    let capacity = values.capacity();
                    let token_bytes = values.capacity.as_ref().map(ImportReservation::bytes);
                    FAIL_NEXT_METERED_QUALIFICATION.with(|fault| fault.set(true));
                    let result = values.try_reserve(capacity);
                    assert_eq!(values.get(&3), Some(&11));
                    assert_eq!(values.len(), 1);
                    assert_eq!(values.capacity(), capacity);
                    assert_eq!(
                        values.capacity.as_ref().map(ImportReservation::bytes),
                        token_bytes
                    );
                    parse_failure(result)
                }
            };
            Err(failure)
        }
    }

    impl FormatImporter for ScenePartsImporter {
        fn descriptor(&self) -> FormatDescriptor {
            FormatDescriptor::builder(
                FormatId::new("dev.baozi.test.scene-parts").expect("valid test format id"),
                "Scene Parts Test",
            )
            .extensions(["scene-parts"])
            .encoding(FormatEncoding::Binary)
            .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
            .build()
            .expect("valid test descriptor")
        }

        fn detect(
            &self,
            _probe: &crate::Probe<'_>,
            _hint: &ReadHint,
            _scope: &DetectionScope<'_>,
        ) -> DetectionResult<DetectionOutcome> {
            Ok(DetectionOutcome::Match(ReadConfidence::Certain))
        }

        fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
            let allowance = scope.metered_opaque_allowance(ImportStorageClass::BackendDecode, 7)?;
            assert_eq!(allowance.bytes(), 7);
            drop(allowance);

            let mut decoded = scope.metered_decoded_text(7, "scene-parts-decoded-text")?;
            decoded.try_push_str("fixture")?;
            let decoded = decoded.finish_exact()?;
            assert_eq!(decoded.as_str(), "fixture");
            assert_eq!(decoded.decoded_bytes(), 7);
            assert_eq!(
                decoded
                    .decoded_bytes()
                    .checked_add(decoded.allocator_slack_bytes()),
                u64::try_from(decoded.value.capacity()).ok()
            );
            drop(decoded);

            let mut name = scope.metered_string(
                ImportStorageClass::SceneConstruction,
                "scene-parts-mesh-name",
            );
            name.try_push_str("metered mesh ")?;
            name.try_push_usize(1)?;
            name.try_push_char('!')?;
            let mut positions = scope.metered_vec(
                ImportStorageClass::SceneConstruction,
                "scene-parts-positions",
            );
            positions.try_extend_from_slice(&[
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ])?;

            let mut texcoords = scope.metered_scene_vec("scene-parts-texcoords");
            for channel in [
                [
                    Vec2::new(0.0, 0.0),
                    Vec2::new(1.0, 0.0),
                    Vec2::new(0.0, 1.0),
                ],
                [
                    Vec2::new(0.5, 0.5),
                    Vec2::new(1.0, 0.5),
                    Vec2::new(0.5, 1.0),
                ],
            ] {
                let mut values = scope.metered_vec(
                    ImportStorageClass::SceneConstruction,
                    "scene-parts-texcoord-channel",
                );
                values.try_extend_from_slice(&channel)?;
                texcoords.try_push_parts(values.try_into_scene_parts()?)?;
            }

            let mut metadata = scope.metered_metadata_map("scene-parts-metadata");
            let mut key = scope.metered_string(
                ImportStorageClass::SceneConstruction,
                "scene-parts-metadata-key",
            );
            key.try_push_str("source:label")?;
            let mut value = scope.metered_string(
                ImportStorageClass::SceneConstruction,
                "scene-parts-metadata-value",
            );
            value.try_push_str("original")?;
            metadata.try_insert_parts(
                key.try_into_scene_parts()?
                    .zip(value.try_into_scene_parts()?)?
                    .into_metadata_string_entry(),
            )?;

            let mut replacement_key = scope.metered_string(
                ImportStorageClass::SceneConstruction,
                "scene-parts-metadata-replacement-key",
            );
            replacement_key.try_push_str("source:label")?;
            let mut replacement_value = scope.metered_string(
                ImportStorageClass::SceneConstruction,
                "scene-parts-metadata-replacement-value",
            );
            replacement_value.try_push_str("replacement")?;
            metadata.try_insert_parts(
                replacement_key
                    .try_into_scene_parts()?
                    .zip(replacement_value.try_into_scene_parts()?)?
                    .into_metadata_string_entry(),
            )?;

            let mut numeric_key = scope.metered_string(
                ImportStorageClass::SceneConstruction,
                "scene-parts-numeric-metadata-key",
            );
            numeric_key.try_push_str("source:index")?;
            metadata.try_insert_parts(
                numeric_key
                    .try_into_scene_parts()?
                    .into_metadata_i64_entry(1),
            )?;

            let prepared = name
                .try_into_scene_parts()?
                .into_some()
                .into_mesh(MeshBuildConfig {
                    topology: PrimitiveTopology::Triangles,
                    material: None,
                    bounds: None,
                })
                .with_positions(positions.try_into_scene_parts()?)?
                .with_texcoords(texcoords.try_into_scene_parts()?)?
                .with_metadata(metadata.try_into_scene_parts()?)?
                .finish()?;
            let footprint = prepared
                .value
                .as_ref()
                .expect("prepared mesh value")
                .scene_value_footprint()
                .expect("prepared mesh footprint")
                .payload_bytes();
            assert_eq!(
                reservation_bytes(&prepared.reservations.payload),
                Some(footprint)
            );
            let mut builder = scope.scene_builder()?;
            let mesh = builder.add_prepared_mesh(prepared)?;
            assert_eq!(mesh, MeshId::new(0));
            builder.finish_draft()
        }
    }

    impl FormatImporter for PreparedMetadataImporter {
        fn descriptor(&self) -> FormatDescriptor {
            FormatDescriptor::builder(
                FormatId::new("dev.baozi.test.prepared-metadata").expect("valid test format id"),
                "Prepared Metadata Test",
            )
            .extensions(["prepared-metadata"])
            .encoding(FormatEncoding::Binary)
            .capabilities([(FormatCapability::Metadata, CapabilityStatus::Supported)])
            .build()
            .expect("valid test descriptor")
        }

        fn detect(
            &self,
            _probe: &crate::Probe<'_>,
            _hint: &ReadHint,
            _scope: &DetectionScope<'_>,
        ) -> DetectionResult<DetectionOutcome> {
            Ok(DetectionOutcome::Match(ReadConfidence::Certain))
        }

        fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
            let mut key = scope.metered_string(
                ImportStorageClass::SceneConstruction,
                "prepared-metadata-key",
            );
            key.try_push_str("source:label")?;
            let mut value = scope.metered_string(
                ImportStorageClass::SceneConstruction,
                "prepared-metadata-value",
            );
            value.try_push_str("fixture")?;
            let prepared = key
                .try_into_scene_parts()?
                .zip(value.try_into_scene_parts()?)?
                .into_metadata_string_entry()
                .finish()?;
            let mut builder = scope.scene_builder()?;
            builder.insert_prepared_metadata(prepared)?;

            let mut replacement_key = scope.metered_string(
                ImportStorageClass::SceneConstruction,
                "prepared-metadata-replacement-key",
            );
            replacement_key.try_push_str("source:label")?;
            let mut replacement_value = scope.metered_string(
                ImportStorageClass::SceneConstruction,
                "prepared-metadata-replacement-value",
            );
            replacement_value.try_push_str("replacement")?;
            let replacement = replacement_key
                .try_into_scene_parts()?
                .zip(replacement_value.try_into_scene_parts()?)?
                .into_metadata_string_entry()
                .finish()?;
            builder.insert_prepared_metadata(replacement)?;
            builder.finish_draft()
        }
    }

    impl FormatImporter for PreparedNodeImporter {
        fn descriptor(&self) -> FormatDescriptor {
            FormatDescriptor::builder(
                FormatId::new("dev.baozi.test.prepared-node").expect("valid test format id"),
                "Prepared Node Test",
            )
            .extensions(["prepared-node"])
            .encoding(FormatEncoding::Binary)
            .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
            .build()
            .expect("valid test descriptor")
        }

        fn detect(
            &self,
            _probe: &crate::Probe<'_>,
            _hint: &ReadHint,
            _scope: &DetectionScope<'_>,
        ) -> DetectionResult<DetectionOutcome> {
            Ok(DetectionOutcome::Match(ReadConfidence::Certain))
        }

        fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
            let mut name =
                scope.metered_string(ImportStorageClass::SceneConstruction, "prepared-node-name");
            name.try_push_str("metered node")?;
            let mut prepared = name
                .try_into_scene_parts()?
                .into_some()
                .into_node(NodeBuildConfig {
                    transform: Mat4::IDENTITY,
                    camera: None,
                    light: None,
                })
                .finish()?;
            let mut builder = scope.scene_builder()?;

            if matches!(self.case, PreparedNodeCase::Success) {
                let node = builder.add_prepared_node(prepared)?;
                assert_eq!(node, NodeId::new(1));
                return builder.finish_draft();
            }

            let node = prepared
                .value
                .as_mut()
                .expect("the prepared node remains owned before insertion");
            match self.case {
                PreparedNodeCase::Success => unreachable!("success returned above"),
                PreparedNodeCase::PreownedParent => node.parent = Some(builder.root()),
                PreparedNodeCase::PreownedChild => node.children.push(builder.root()),
                PreparedNodeCase::PreownedMeshBinding => {
                    node.mesh_bindings.push(MeshBinding::new(MeshId::new(0)));
                }
            }
            let prepared_live_bytes = prepared
                .reservations
                .payload
                .capacity_bytes()
                .checked_add(
                    reservation_bytes(&prepared.reservations.payload)
                        .expect("prepared node reservation bytes"),
                )
                .expect("prepared node live bytes fit u64");
            let live_before = scope
                .coordinator
                .borrow()
                .try_ledger_snapshot()
                .expect("pre-rejection ledger snapshot")
                .current_live_bytes();
            let failure = builder
                .add_prepared_node(prepared)
                .expect_err("a prepared node cannot pre-own scene relationships");
            let live_after = scope
                .coordinator
                .borrow()
                .try_ledger_snapshot()
                .expect("post-rejection ledger snapshot")
                .current_live_bytes();
            assert_eq!(
                live_before.checked_sub(live_after),
                Some(prepared_live_bytes),
                "rejection must release the complete prepared-node owner"
            );
            Err(failure)
        }
    }

    fn parse_failure(result: ParseResult<()>) -> ParseFailure {
        match result {
            Ok(()) => panic!("the injected qualification failure must be observed"),
            Err(failure) => failure,
        }
    }

    fn reserve_test_storage(
        coordinator: &RefCell<ImportCoordinator>,
        bytes: u64,
    ) -> Result<ImportReservation, ScopeAbortCause> {
        coordinator
            .borrow_mut()
            .checkpoint()
            .reserve(
                ImportLedgerPhase::SceneConstruction,
                ImportDimension::ScratchBytes,
                bytes,
            )
            .map_err(ScopeAbortCause::Ledger)
    }

    fn fill_reservation_lane(
        coordinator: &RefCell<ImportCoordinator>,
        lane: &mut AccountedReservationVec,
        payload_bytes: u64,
    ) {
        lane.try_reserve(1, "merge-fixture-capacity", |bytes| {
            reserve_test_storage(coordinator, bytes)
        })
        .expect("merge fixture capacity must be admitted");
        while lane.len() < lane.capacity() {
            let reservation = reserve_test_storage(coordinator, payload_bytes)
                .expect("merge fixture payload must be admitted");
            lane.try_push_reserved(reservation)
                .unwrap_or_else(|_| panic!("merge fixture slot must remain available"));
        }
    }

    fn fill_reservation_inventory(
        coordinator: &RefCell<ImportCoordinator>,
        inventory: &mut SceneValueReservations,
    ) {
        fill_reservation_lane(coordinator, &mut inventory.payload, 11);
    }

    fn reservation_inventory_state(
        inventory: &SceneValueReservations,
    ) -> (usize, usize, u64, Vec<u64>) {
        (
            inventory.payload.len(),
            inventory.payload.capacity(),
            inventory.payload.capacity_bytes(),
            inventory
                .payload
                .iter()
                .map(ImportReservation::bytes)
                .collect(),
        )
    }

    fn assert_not_send_sync<T: ?Sized>() {
        let _ = std::any::type_name::<T>();
    }

    #[test]
    fn parse_scope_is_intentionally_session_local() {
        assert_not_send_sync::<ParseScope<'_>>();
    }

    #[test]
    fn metered_growth_qualification_failure_preserves_every_live_owner() {
        for container in [
            QualificationContainer::Vec,
            QualificationContainer::String,
            QualificationContainer::OrderedMap,
        ] {
            let mut registry = RegistryBuilder::new();
            registry
                .register(QualificationFailureImporter { container })
                .expect("the test importer must register");
            let snapshot = registry
                .freeze(RegistryBuildLimits::default())
                .expect("the test registry must freeze");
            let request = snapshot
                .compile_request(&ImportRequest::new(), OptionCompileLimits::default())
                .expect("the test request must compile");
            let result = crate::session::import_bytes(
                &request,
                AssetPath::new("asset.metered-qualification").expect("valid test path"),
                b"fixture",
                ImportOperationControl::default(),
            );
            assert!(
                result.is_err(),
                "the injected allocator qualification failure must terminate the import"
            );
        }
    }

    #[test]
    fn scene_parts_keep_storage_and_tokens_coupled_through_terminal_composition() {
        let mut registry = RegistryBuilder::new();
        registry
            .register(ScenePartsImporter)
            .expect("the scene-parts importer must register");
        let snapshot = registry
            .freeze(RegistryBuildLimits::default())
            .expect("the scene-parts registry must freeze");
        let request = snapshot
            .compile_request(&ImportRequest::new(), OptionCompileLimits::default())
            .expect("the scene-parts request must compile");
        let result = crate::session::import_bytes(
            &request,
            AssetPath::new("asset.scene-parts").expect("valid test path"),
            b"fixture",
            ImportOperationControl::default(),
        );
        assert!(
            result.is_ok(),
            "scene-parts composition must remain importable"
        );
    }

    #[test]
    fn prepared_metadata_keeps_key_value_and_map_ownership_coupled() {
        let mut registry = RegistryBuilder::new();
        registry
            .register(PreparedMetadataImporter)
            .expect("the prepared-metadata importer must register");
        let snapshot = registry
            .freeze(RegistryBuildLimits::default())
            .expect("the prepared-metadata registry must freeze");
        let request = snapshot
            .compile_request(&ImportRequest::new(), OptionCompileLimits::default())
            .expect("the prepared-metadata request must compile");
        let result = crate::session::import_bytes(
            &request,
            AssetPath::new("asset.prepared-metadata").expect("valid test path"),
            b"fixture",
            ImportOperationControl::default(),
        );
        assert!(
            result.is_ok(),
            "prepared metadata must retain its key and value reservations through publication"
        );
    }

    #[test]
    fn prepared_node_transfers_metered_payload_into_the_scene() {
        let result = import_prepared_node(PreparedNodeCase::Success);
        assert!(
            result.is_ok(),
            "a relationship-free prepared node must publish"
        );
    }

    #[test]
    fn prepared_node_rejects_every_preowned_relationship_without_leaking_storage() {
        for case in [
            PreparedNodeCase::PreownedParent,
            PreparedNodeCase::PreownedChild,
            PreparedNodeCase::PreownedMeshBinding,
        ] {
            let result = import_prepared_node(case);
            assert!(
                result.is_err(),
                "a prepared node with a preowned relationship must fail"
            );
        }
    }

    #[allow(
        clippy::result_large_err,
        reason = "the test exercises the public terminal outcome without adding heap indirection"
    )]
    fn import_prepared_node(case: PreparedNodeCase) -> crate::ImportResult<crate::ImportReport> {
        let mut registry = RegistryBuilder::new();
        registry
            .register(PreparedNodeImporter { case })
            .expect("the prepared-node importer must register");
        let snapshot = registry
            .freeze(RegistryBuildLimits::default())
            .expect("the prepared-node registry must freeze");
        let request = snapshot
            .compile_request(&ImportRequest::new(), OptionCompileLimits::default())
            .expect("the prepared-node request must compile");
        crate::session::import_bytes(
            &request,
            AssetPath::new("asset.prepared-node").expect("valid test path"),
            b"fixture",
            ImportOperationControl::default(),
        )
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

        let snapshot = read_opened_asset(
            &coordinator,
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
        assert!(
            coordinator
                .borrow()
                .try_ledger_snapshot()
                .expect("active test ledger snapshot")
                .current_live_bytes()
                > 0
        );

        drop(snapshot);
        assert_eq!(
            coordinator
                .borrow()
                .try_ledger_snapshot()
                .expect("active test ledger snapshot")
                .current_live_bytes(),
            0
        );
    }

    struct OwnedIterationDropProbe<'a> {
        id: u8,
        coordinator: &'a RefCell<ImportCoordinator>,
        observations: &'a RefCell<Vec<(u8, bool)>>,
    }

    impl Drop for OwnedIterationDropProbe<'_> {
        fn drop(&mut self) {
            let capacity_is_live = self
                .coordinator
                .borrow()
                .try_ledger_snapshot()
                .expect("owned-iteration drop-order snapshot")
                .dimension(ImportDimension::CanonicalOutputBytes)
                .expect("canonical-output dimension")
                .current_live_bytes()
                > 0;
            self.observations
                .borrow_mut()
                .push((self.id, capacity_is_live));
        }
    }

    fn assert_owned_iteration_drop_observations(
        observations: &RefCell<Vec<(u8, bool)>>,
        coordinator: &RefCell<ImportCoordinator>,
    ) {
        let mut observations = observations.borrow().clone();
        observations.sort_unstable_by_key(|(id, _)| *id);
        assert_eq!(
            observations,
            vec![(0, true), (1, true), (2, true)],
            "the current and remaining elements must all drop under the live capacity reservation"
        );
        assert_eq!(
            coordinator
                .borrow()
                .try_ledger_snapshot()
                .expect("released owned-iteration snapshot")
                .current_live_bytes(),
            0
        );
    }

    #[test]
    fn owned_iteration_error_drops_current_and_remaining_elements_before_capacity() {
        let limits = ResourceLimits::default();
        let coordinator = RefCell::new(ImportCoordinator::new(
            ImportOperationControl::default(),
            import_ledger_limits(&limits).expect("valid import ledger limits"),
        ));
        let terminal = RefCell::new(None);
        let meter = StorageMeter {
            coordinator: &coordinator,
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
                    coordinator: &coordinator,
                    observations: &observations,
                })
                .expect("owned-iteration fixture capacity");
        }

        let result = values.try_for_each_owned(|probe| {
            assert_eq!(probe.id, 0, "the first visitor call must inject failure");
            Err::<(), _>("injected visitor failure")
        });

        assert_eq!(result, Err("injected visitor failure"));
        assert_owned_iteration_drop_observations(&observations, &coordinator);
    }

    #[test]
    fn owned_iteration_panic_drops_current_and_remaining_elements_before_capacity() {
        let limits = ResourceLimits::default();
        let coordinator = RefCell::new(ImportCoordinator::new(
            ImportOperationControl::default(),
            import_ledger_limits(&limits).expect("valid import ledger limits"),
        ));
        let terminal = RefCell::new(None);
        let meter = StorageMeter {
            coordinator: &coordinator,
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
                    coordinator: &coordinator,
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
        assert_owned_iteration_drop_observations(&observations, &coordinator);
    }

    #[test]
    fn reservation_merge_failure_preserves_both_payload_owners() {
        let limits = ResourceLimits::default();
        let coordinator = RefCell::new(ImportCoordinator::new(
            ImportOperationControl::default(),
            import_ledger_limits(&limits).expect("valid import ledger limits"),
        ));
        let mut target = SceneValueReservations::new();
        let mut source = SceneValueReservations::new();
        fill_reservation_inventory(&coordinator, &mut target);
        fill_reservation_inventory(&coordinator, &mut source);
        let target_before = reservation_inventory_state(&target);
        let source_before = reservation_inventory_state(&source);
        let live_before = coordinator
            .borrow()
            .try_ledger_snapshot()
            .expect("pre-merge snapshot")
            .current_live_bytes();
        let mut calls = 0usize;

        let failure = target
            .try_merge_preserving(&mut source, "merge-failure-fixture", |_| {
                calls += 1;
                Err(ScopeAbortCause::Allocation {
                    resource: "merge-failure-fixture",
                })
            })
            .expect_err("the injected payload-lane admission must fail");

        assert!(matches!(
            failure,
            ScopeAbortCause::Allocation {
                resource: "merge-failure-fixture"
            }
        ));
        assert_eq!(calls, 1);
        assert_eq!(reservation_inventory_state(&target), target_before);
        assert_eq!(reservation_inventory_state(&source), source_before);
        assert_eq!(
            coordinator
                .borrow()
                .try_ledger_snapshot()
                .expect("post-merge snapshot")
                .current_live_bytes(),
            live_before
        );

        drop(target);
        drop(source);
        let ledger = coordinator
            .into_inner()
            .finish()
            .expect("preserved merge owners must finalize cleanly");
        assert_eq!(ledger.current_live_bytes(), 0);
    }

    #[test]
    fn scene_parts_finalizer_drops_value_before_current_and_remaining_reservations_on_panic() {
        struct FinalizationDropProbe<'a> {
            coordinator: &'a RefCell<ImportCoordinator>,
            observed_current_reservation: &'a std::cell::Cell<bool>,
            observed_remaining_reservation: &'a std::cell::Cell<bool>,
        }

        impl Drop for FinalizationDropProbe<'_> {
            fn drop(&mut self) {
                let snapshot = self
                    .coordinator
                    .borrow()
                    .try_ledger_snapshot()
                    .expect("drop-order snapshot");
                self.observed_current_reservation.set(
                    snapshot
                        .dimension(ImportDimension::DecodedTextBytes)
                        .expect("decoded-text dimension")
                        .current_live_bytes()
                        == 7,
                );
                self.observed_remaining_reservation.set(
                    snapshot
                        .dimension(ImportDimension::LineIndexBytes)
                        .expect("line-index dimension")
                        .current_live_bytes()
                        == 11,
                );
            }
        }

        fn entry_owner(
            coordinator: &RefCell<ImportCoordinator>,
            resource: &'static str,
            phase: ImportLedgerPhase,
            dimension: ImportDimension,
            bytes: u64,
        ) -> SceneValueReservations {
            let mut entry = SceneValueReservations::new();
            entry
                .payload
                .try_reserve(1, resource, |capacity_bytes| {
                    reserve_test_storage(coordinator, capacity_bytes)
                })
                .expect("entry reservation capacity");
            entry
                .payload
                .try_push_reserved(
                    coordinator
                        .borrow_mut()
                        .checkpoint()
                        .reserve(phase, dimension, bytes)
                        .expect("entry payload reservation"),
                )
                .unwrap_or_else(|_| panic!("entry reservation slot"));
            entry
        }

        let limits = ResourceLimits::default();
        let coordinator = RefCell::new(ImportCoordinator::new(
            ImportOperationControl::default(),
            import_ledger_limits(&limits).expect("valid import ledger limits"),
        ));
        let terminal = RefCell::new(None);
        let meter = StorageMeter {
            coordinator: &coordinator,
            terminal: &terminal,
        };
        let observed_current_reservation = std::cell::Cell::new(false);
        let observed_remaining_reservation = std::cell::Cell::new(false);
        let parts = SceneValueParts {
            value: Some(FinalizationDropProbe {
                coordinator: &coordinator,
                observed_current_reservation: &observed_current_reservation,
                observed_remaining_reservation: &observed_remaining_reservation,
            }),
            reservations: SceneValueReservations::new(),
            meter,
        };
        let remaining_entry = entry_owner(
            &coordinator,
            "finalizer-remaining-entry-capacity",
            ImportLedgerPhase::FormatNormalization,
            ImportDimension::LineIndexBytes,
            11,
        );
        let current_entry = entry_owner(
            &coordinator,
            "finalizer-current-entry-capacity",
            ImportLedgerPhase::BackendDecode,
            ImportDimension::DecodedTextBytes,
            7,
        );
        let mut entries = MeteredVec {
            values: Vec::new(),
            capacity: None,
            meter,
            class: ImportStorageClass::SceneConstruction,
            resource: "finalizer-entry-owners",
        };
        entries
            .try_push(remaining_entry)
            .expect("remaining entry owner capacity");
        entries
            .try_push(current_entry)
            .expect("current entry owner capacity");

        PANIC_NEXT_SCENE_PARTS_FINALIZATION.with(|fault| fault.set(true));
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _ = ScenePartsReservationFinalizer {
                parts,
                current: None,
                entries,
            }
            .try_finish();
        }));
        assert!(result.is_err(), "the finalization panic must be injected");
        assert!(
            observed_current_reservation.get(),
            "the canonical value must drop while the current entry reservation is still live"
        );
        assert!(
            observed_remaining_reservation.get(),
            "the canonical value must drop while the remaining entry reservation is still live"
        );
        assert_eq!(
            coordinator
                .borrow()
                .try_ledger_snapshot()
                .expect("released finalizer snapshot")
                .current_live_bytes(),
            0
        );
    }

    #[test]
    fn reserved_vector_copy_rejects_an_unbounded_chunk_without_mutation() {
        let limits = ResourceLimits::default();
        let coordinator = RefCell::new(ImportCoordinator::new(
            ImportOperationControl::default(),
            import_ledger_limits(&limits).expect("valid import ledger limits"),
        ));
        let terminal = RefCell::new(None);
        let meter = StorageMeter {
            coordinator: &coordinator,
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
        let coordinator = RefCell::new(ImportCoordinator::new(
            ImportOperationControl::default(),
            import_ledger_limits(&limits).expect("valid import ledger limits"),
        ));
        let terminal = RefCell::new(None);
        let meter = StorageMeter {
            coordinator: &coordinator,
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
        let coordinator = RefCell::new(ImportCoordinator::new(
            ImportOperationControl::default(),
            import_ledger_limits(&limits).expect("valid import ledger limits"),
        ));
        let terminal = RefCell::new(None);
        let meter = StorageMeter {
            coordinator: &coordinator,
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
