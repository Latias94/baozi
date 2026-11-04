use crate::dependency::DependencyOriginIssuer;
use crate::error::ScopeAbortCause;
use crate::ledger::ImportDimension;
use crate::metered_storage::AccountedVec;
use crate::scope::{
    ImportStorageClass, METERED_COPY_POLL_INTERVAL, ParseFailure, ParseResult, ParseServices,
};
use baozi_io::AssetPath;
use sha2::{Digest, Sha256};
use std::fmt;

const SOURCE_SCOPE_DIGEST_DOMAIN: &[u8] = b"baozi.source-scope.v1\0";
const SOURCE_SCOPE_SELECTION_FRAME_TAG: u8 = 0xff;

/// A source-document entity category counted by source-scope evidence.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum SourceScopeEntityKind {
    Scene = 0,
    Node = 1,
    Mesh = 2,
    Primitive = 3,
    Material = 4,
    Texture = 5,
    Image = 6,
    Sampler = 7,
    Skin = 8,
    Animation = 9,
    AnimationChannel = 10,
    Buffer = 11,
    BufferView = 12,
    Accessor = 13,
    /// A camera declared by the source document.
    Camera = 14,
    /// An animation sampler scoped to its source animation.
    AnimationSampler = 15,
    /// A reusable light definition; node observations describe its placements.
    Light = 16,
}

impl SourceScopeEntityKind {
    const COUNT: usize = Self::Light as usize + 1;

    const fn index(self) -> usize {
        self as usize
    }
}

/// Whether a source entity belongs to the selected import closure.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceScopeDisposition {
    Selected = 1,
    Omitted = 2,
}

/// A sanitized source-document identity used by selection evidence.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceScopeDocumentIdentity(AssetPath);

impl SourceScopeDocumentIdentity {
    pub fn new(document: AssetPath) -> Self {
        Self(document)
    }

    pub const fn asset_path(&self) -> &AssetPath {
        &self.0
    }
}

/// A format-owned identity for one selected source object.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceScopeSourceIdentity {
    document: SourceScopeDocumentIdentity,
    scene_index: u64,
}

impl SourceScopeSourceIdentity {
    pub fn new(document: SourceScopeDocumentIdentity, scene_index: u64) -> Self {
        Self {
            document,
            scene_index,
        }
    }

    pub const fn document(&self) -> &SourceScopeDocumentIdentity {
        &self.document
    }

    pub const fn scene_index(&self) -> u64 {
        self.scene_index
    }
}

/// Requested mode retained as evidence after format-owned option resolution.
///
/// This is report evidence, not a format-independent import option. An explicit
/// source scene index is interpreted only by the format that owns the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceScopeSelectionRequest {
    /// Resolve the document's default-selection policy.
    DocumentDefault,
    /// Select one declared scene by its original source index.
    ExplicitScene { scene_index: u64 },
}

/// Typed result of the format-owned source selection step.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SourceScopeSelectionOutcome {
    /// A declared source scene was selected.
    DeclaredScene(SourceScopeSourceIdentity),
    /// No source scene was declared; parentless nodes define the imported projection.
    ParentlessNodeProjection(SourceScopeDocumentIdentity),
}

impl SourceScopeSelectionOutcome {
    pub fn declared_scene(document: AssetPath, scene_index: u64) -> Self {
        Self::DeclaredScene(SourceScopeSourceIdentity::new(
            SourceScopeDocumentIdentity::new(document),
            scene_index,
        ))
    }

    pub fn parentless_node_projection(document: AssetPath) -> Self {
        Self::ParentlessNodeProjection(SourceScopeDocumentIdentity::new(document))
    }

    pub const fn document(&self) -> &SourceScopeDocumentIdentity {
        match self {
            Self::DeclaredScene(identity) => identity.document(),
            Self::ParentlessNodeProjection(document) => document,
        }
    }

    pub const fn scene_index(&self) -> Option<u64> {
        match self {
            Self::DeclaredScene(identity) => Some(identity.scene_index()),
            Self::ParentlessNodeProjection(_) => None,
        }
    }
}

/// Retention state for optional per-entity source-scope detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceScopeDetailState {
    Detailed,
    Collapsed,
}

/// Exact selected and omitted counts for one typed source category.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct SourceScopeCounts {
    selected: u64,
    omitted: u64,
}

impl SourceScopeCounts {
    pub const fn selected(self) -> u64 {
        self.selected
    }

    pub const fn omitted(self) -> u64 {
        self.omitted
    }

    pub const fn total(self) -> u64 {
        self.selected.saturating_add(self.omitted)
    }

    fn increment(&mut self, disposition: SourceScopeDisposition) -> Result<(), ()> {
        let count = match disposition {
            SourceScopeDisposition::Selected => &mut self.selected,
            SourceScopeDisposition::Omitted => &mut self.omitted,
        };
        *count = count.checked_add(1).ok_or(())?;
        Ok(())
    }
}

/// SHA-256 over the complete ordered source-scope observation stream.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceScopeDigest([u8; 32]);

impl SourceScopeDigest {
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for SourceScopeDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("SourceScopeDigest")
            .field(&self.0)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceScopeDetailIndex {
    kind: SourceScopeEntityKind,
    disposition: SourceScopeDisposition,
    start: usize,
    len: usize,
}

/// One borrowed detailed source-scope fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceScopeDetail<'record> {
    kind: SourceScopeEntityKind,
    disposition: SourceScopeDisposition,
    identity: &'record str,
}

impl<'record> SourceScopeDetail<'record> {
    pub const fn kind(self) -> SourceScopeEntityKind {
        self.kind
    }

    pub const fn disposition(self) -> SourceScopeDisposition {
        self.disposition
    }

    pub const fn identity(self) -> &'record str {
        self.identity
    }
}

/// Ordered borrowed iteration over retained source-scope details.
pub struct SourceScopeDetails<'record> {
    record: &'record SourceScopeRecord,
    next: usize,
}

impl<'record> Iterator for SourceScopeDetails<'record> {
    type Item = SourceScopeDetail<'record>;

    fn next(&mut self) -> Option<Self::Item> {
        let index = self.record.detail_indexes.get(self.next)?;
        self.next += 1;
        let end = index.start.checked_add(index.len)?;
        let identity = self.record.detail_bytes.get(index.start..end)?;
        let identity = std::str::from_utf8(identity).ok()?;
        Some(SourceScopeDetail {
            kind: index.kind,
            disposition: index.disposition,
            identity,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.record.detail_indexes.len().saturating_sub(self.next);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for SourceScopeDetails<'_> {}

/// Baozi-owned inert evidence describing one selected source closure.
pub struct SourceScopeRecord {
    selection_request: SourceScopeSelectionRequest,
    selection_outcome: SourceScopeSelectionOutcome,
    counts: [SourceScopeCounts; SourceScopeEntityKind::COUNT],
    digest: SourceScopeDigest,
    detail_state: SourceScopeDetailState,
    detail_indexes: AccountedVec<SourceScopeDetailIndex>,
    detail_bytes: AccountedVec<u8>,
    collapsed_detail_count: u64,
    collapsed_detail_bytes: u64,
    _document_storage: crate::store::ImportStoreReservation,
}

impl SourceScopeRecord {
    /// Returns the request that produced this projection, including after detail collapse.
    pub const fn selection_request(&self) -> &SourceScopeSelectionRequest {
        &self.selection_request
    }

    /// Returns the resolved source identity under the requested selection mode.
    pub fn selection_outcome(&self) -> &SourceScopeSelectionOutcome {
        &self.selection_outcome
    }
    pub const fn counts(&self, kind: SourceScopeEntityKind) -> SourceScopeCounts {
        self.counts[kind.index()]
    }

    pub const fn digest(&self) -> SourceScopeDigest {
        self.digest
    }

    pub const fn detail_state(&self) -> SourceScopeDetailState {
        self.detail_state
    }

    pub const fn collapsed_detail_count(&self) -> u64 {
        self.collapsed_detail_count
    }

    pub const fn collapsed_detail_bytes(&self) -> u64 {
        self.collapsed_detail_bytes
    }

    pub fn details(&self) -> SourceScopeDetails<'_> {
        SourceScopeDetails {
            record: self,
            next: 0,
        }
    }
}

impl fmt::Debug for SourceScopeRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SourceScopeRecord")
            .field("selection_request", &self.selection_request)
            .field("selection_outcome", &self.selection_outcome)
            .field("counts", &self.counts)
            .field("digest", &self.digest)
            .field("detail_state", &self.detail_state)
            .field("detail_count", &self.detail_indexes.len())
            .field("detail_bytes", &self.detail_bytes.len())
            .field("collapsed_detail_count", &self.collapsed_detail_count)
            .field("collapsed_detail_bytes", &self.collapsed_detail_bytes)
            .finish()
    }
}

/// Incremental, session-metered construction of one source-scope record.
///
/// Finalized evidence can be submitted through a fresh short service borrow:
/// ```
/// use baozi_import::{ParseResult, ParseRoot, SourceScopeRecordBuilder};
/// fn submit<'session>(
///     root: &mut ParseRoot<'_, '_, 'session>,
///     builder: SourceScopeRecordBuilder<'session>,
/// ) -> ParseResult<()> {
///     let finalized = builder.finish(&mut root.services()?)?;
///     root.services()?.record_source_scope(finalized)
/// }
/// ```
///
/// A live service borrow excludes another service borrow from the same root:
/// ```compile_fail,E0499
/// use baozi_import::{ParseResult, ParseRoot, SourceScopeRecordBuilder};
/// fn submit<'session>(
///     root: &mut ParseRoot<'_, '_, 'session>,
///     builder: SourceScopeRecordBuilder<'session>,
/// ) -> ParseResult<()> {
///     let mut services = root.services()?;
///     let finalized = builder.finish(&mut services)?;
///     root.services()?.record_source_scope(finalized)?;
///     services.poll()
/// }
/// ```
#[must_use = "finish and submit source-scope evidence or drop the builder"]
pub struct SourceScopeRecordBuilder<'session> {
    issuer: &'session DependencyOriginIssuer,
    selection_request: SourceScopeSelectionRequest,
    selection_outcome: SourceScopeSelectionOutcome,
    counts: [SourceScopeCounts; SourceScopeEntityKind::COUNT],
    digest: Sha256,
    detail_state: SourceScopeDetailState,
    detail_indexes: AccountedVec<SourceScopeDetailIndex>,
    detail_bytes: AccountedVec<u8>,
    observed_count: u64,
    observed_bytes: u64,
    _document_storage: crate::store::ImportStoreReservation,
}

impl<'session> SourceScopeRecordBuilder<'session> {
    pub(crate) fn new(
        scope: &mut ParseServices<'_, '_, 'session>,
        selection_request: SourceScopeSelectionRequest,
        selection_outcome: SourceScopeSelectionOutcome,
    ) -> ParseResult<Self> {
        // This argument transfers already-owned caller storage. Admit its actual backing capacity
        // before adopting it into the library-owned builder and eventual output record.
        let document_bytes = selection_outcome.document().asset_path().capacity_bytes() as u64;
        let _document_storage = scope
            .reserve_retained_storage_cause(
                ImportStorageClass::FormatNormalization,
                document_bytes,
                u64::from(document_bytes != 0),
            )
            .map_err(|cause| scope.source_scope_abort(cause))?;
        let mut digest = Sha256::new();
        digest.update(SOURCE_SCOPE_DIGEST_DOMAIN);
        digest.update(SOURCE_SCOPE_SELECTION_FRAME_TAG.to_le_bytes());
        match selection_request {
            SourceScopeSelectionRequest::DocumentDefault => digest.update([0]),
            SourceScopeSelectionRequest::ExplicitScene { scene_index } => {
                digest.update([1]);
                digest.update(scene_index.to_le_bytes());
            }
        }
        match &selection_outcome {
            SourceScopeSelectionOutcome::DeclaredScene(identity) => {
                digest.update([1]);
                digest.update(identity.scene_index().to_le_bytes());
                Self::digest_document(&mut digest, identity.document().asset_path());
            }
            SourceScopeSelectionOutcome::ParentlessNodeProjection(document) => {
                digest.update([2]);
                Self::digest_document(&mut digest, document.asset_path());
            }
        }
        Ok(Self {
            issuer: scope.session_issuer(),
            selection_request,
            selection_outcome,
            counts: [SourceScopeCounts::default(); SourceScopeEntityKind::COUNT],
            digest,
            detail_state: SourceScopeDetailState::Detailed,
            detail_indexes: AccountedVec::new(),
            detail_bytes: AccountedVec::new(),
            observed_count: 0,
            observed_bytes: 0,
            _document_storage,
        })
    }

    fn digest_document(digest: &mut Sha256, document: &AssetPath) {
        let document_bytes = document.as_str().len() as u64;
        digest.update(document_bytes.to_le_bytes());
        digest.update(document.as_str().as_bytes());
    }

    pub fn observe(
        &mut self,
        scope: &mut ParseServices<'_, '_, 'session>,
        kind: SourceScopeEntityKind,
        disposition: SourceScopeDisposition,
        identity: &str,
    ) -> ParseResult<()> {
        scope.check_session(self.issuer)?;
        scope.poll()?;
        let identity_bytes = u64::try_from(identity.len())
            .map_err(|_| scope.resource_limit_exceeded("source-scope-identity-bytes", u64::MAX))?;
        scope.enforce_resource_limit(
            "source-scope-identity-bytes",
            identity_bytes,
            u64::try_from(scope.limits().max_string_bytes).unwrap_or(u64::MAX),
        )?;
        let observed_count = self
            .observed_count
            .checked_add(1)
            .ok_or_else(|| scope.resource_limit_exceeded("source-scope-items", u64::MAX))?;
        let observed_bytes = self
            .observed_bytes
            .checked_add(identity_bytes)
            .ok_or_else(|| scope.resource_limit_exceeded("source-scope-digest-bytes", u64::MAX))?;
        scope.source_scope_charge_batch(&[
            (ImportDimension::SourceScopeItemCount, 1),
            (ImportDimension::SourceScopeDigestBytes, identity_bytes),
        ])?;

        // Entity framing remains stable: typed tag, disposition, LE length, then full bytes.
        self.digest.update([kind as u8, disposition as u8]);
        self.digest.update(identity_bytes.to_le_bytes());
        for (chunk_index, chunk) in identity
            .as_bytes()
            .chunks(METERED_COPY_POLL_INTERVAL)
            .enumerate()
        {
            if chunk_index != 0 {
                scope.poll()?;
            }
            self.digest.update(chunk);
        }

        self.counts[kind.index()]
            .increment(disposition)
            .map_err(|()| scope.resource_limit_exceeded("source-scope-typed-count", u64::MAX))?;
        self.observed_count = observed_count;
        self.observed_bytes = observed_bytes;

        if self.detail_state == SourceScopeDetailState::Collapsed {
            return Ok(());
        }

        let exceeds_count = observed_count > scope.limits().max_source_scope_detail_count;
        let exceeds_bytes = observed_bytes > scope.limits().max_source_scope_detail_bytes;
        if exceeds_count || exceeds_bytes {
            if !scope
                .loss_policy()
                .allows(crate::ImportLossKind::ProvenanceEvidenceLoss)
            {
                let (resource, limit) = if exceeds_count {
                    (
                        "source-scope-detail-count",
                        scope.limits().max_source_scope_detail_count,
                    )
                } else {
                    (
                        "source-scope-detail-bytes",
                        scope.limits().max_source_scope_detail_bytes,
                    )
                };
                return Err(scope.resource_limit_exceeded(resource, limit));
            }
            self.detail_state = SourceScopeDetailState::Collapsed;
            self.detail_indexes = AccountedVec::new();
            self.detail_bytes = AccountedVec::new();
            return Ok(());
        }

        scope.source_scope_charge_batch(&[
            (ImportDimension::SourceScopeDetailCount, 1),
            (ImportDimension::SourceScopeDetailBytes, identity_bytes),
        ])?;

        self.detail_indexes
            .try_reserve(1, "source-scope-detail-index", |bytes| {
                scope.reserve_retained_storage_cause(
                    ImportStorageClass::FormatNormalization,
                    bytes,
                    u64::from(bytes != 0),
                )
            })
            .map_err(|cause| scope.source_scope_abort(cause))?;
        self.detail_bytes
            .try_reserve(identity.len(), "source-scope-detail-bytes", |bytes| {
                scope.reserve_retained_storage_cause(
                    ImportStorageClass::FormatNormalization,
                    bytes,
                    u64::from(bytes != 0),
                )
            })
            .map_err(|cause| scope.source_scope_abort(cause))?;

        let start = self.detail_bytes.len();
        for chunk in identity.as_bytes().chunks(METERED_COPY_POLL_INTERVAL) {
            scope.poll()?;
            self.detail_bytes
                .try_extend_from_slice_reserved(chunk)
                .map_err(|()| {
                    scope.source_scope_descriptor_failure(
                        "source-scope-detail-capacity-not-admitted",
                    )
                })?;
        }
        self.detail_indexes
            .try_push_reserved(SourceScopeDetailIndex {
                kind,
                disposition,
                start,
                len: identity.len(),
            })
            .map_err(|_| {
                scope.source_scope_descriptor_failure("source-scope-index-capacity-not-admitted")
            })?;
        Ok(())
    }

    pub fn finish(
        self,
        scope: &mut ParseServices<'_, '_, 'session>,
    ) -> ParseResult<FinalizedSourceScopeRecord<'session>> {
        scope.check_session(self.issuer)?;
        let digest = SourceScopeDigest(self.digest.finalize().into());
        Ok(FinalizedSourceScopeRecord {
            issuer: self.issuer,
            record: LiveSourceScopeRecord {
                selection_request: self.selection_request,
                selection_outcome: self.selection_outcome,
                counts: self.counts,
                digest,
                detail_state: self.detail_state,
                detail_indexes: self.detail_indexes,
                detail_bytes: self.detail_bytes,
                collapsed_detail_count: if self.detail_state == SourceScopeDetailState::Collapsed {
                    self.observed_count
                } else {
                    0
                },
                collapsed_detail_bytes: if self.detail_state == SourceScopeDetailState::Collapsed {
                    self.observed_bytes
                } else {
                    0
                },
                _document_storage: self._document_storage,
            },
        })
    }
}

/// Finalized evidence carrying only its independent issuing session identity.
#[must_use = "submit finalized source-scope evidence to its issuing session"]
pub struct FinalizedSourceScopeRecord<'session> {
    issuer: &'session DependencyOriginIssuer,
    record: LiveSourceScopeRecord,
}

impl FinalizedSourceScopeRecord<'_> {
    pub(crate) fn is_issued_by(&self, issuer: &DependencyOriginIssuer) -> bool {
        std::ptr::eq(self.issuer, issuer)
    }

    pub(crate) fn into_record(self) -> LiveSourceScopeRecord {
        self.record
    }
}

pub(crate) struct LiveSourceScopeRecord {
    selection_request: SourceScopeSelectionRequest,
    selection_outcome: SourceScopeSelectionOutcome,
    counts: [SourceScopeCounts; SourceScopeEntityKind::COUNT],
    digest: SourceScopeDigest,
    detail_state: SourceScopeDetailState,
    detail_indexes: AccountedVec<SourceScopeDetailIndex>,
    detail_bytes: AccountedVec<u8>,
    collapsed_detail_count: u64,
    collapsed_detail_bytes: u64,
    _document_storage: crate::store::ImportStoreReservation,
}

impl LiveSourceScopeRecord {
    pub(crate) const fn is_collapsed(&self) -> bool {
        matches!(self.detail_state, SourceScopeDetailState::Collapsed)
    }

    pub(crate) const fn collapsed_detail_count(&self) -> u64 {
        self.collapsed_detail_count
    }

    pub(crate) const fn collapsed_detail_bytes(&self) -> u64 {
        self.collapsed_detail_bytes
    }

    pub(crate) fn into_terminal(self) -> SourceScopeRecord {
        let detail_indexes = self.detail_indexes;
        let detail_bytes = self.detail_bytes;
        SourceScopeRecord {
            selection_request: self.selection_request,
            selection_outcome: self.selection_outcome,
            counts: self.counts,
            digest: self.digest,
            detail_state: self.detail_state,
            detail_indexes,
            detail_bytes,
            collapsed_detail_count: self.collapsed_detail_count,
            collapsed_detail_bytes: self.collapsed_detail_bytes,
            _document_storage: self._document_storage,
        }
    }
}

impl fmt::Debug for FinalizedSourceScopeRecord<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FinalizedSourceScopeRecord")
            .finish_non_exhaustive()
    }
}

impl ParseServices<'_, '_, '_> {
    pub(crate) fn source_scope_descriptor_failure(&self, code: &'static str) -> ParseFailure {
        self.source_scope_abort(ScopeAbortCause::DescriptorContract { code })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metered_storage::inject_next_accounted_allocation_failure;
    use crate::{
        CapabilityStatus, DetectionOutcome, DetectionResult, DetectionScope, Format,
        FormatCapability, FormatDescriptor, FormatEncoding, ImportCause, ImportConfigOverlay,
        ImportLossKind, ImportOperationControl, ImportRequest, ImportRequestLayer, LossPolicy,
        OptionCompileLimits, ReadConfidence, ReadHint, RegistryBuildLimits, RegistryBuilder,
        ResourceLimits, ResourceLimitsOverlay,
    };
    use baozi_core::FormatId;
    use baozi_io::AssetPath;

    struct CollapsedAllocationFailureImporter;

    impl Format for CollapsedAllocationFailureImporter {
        type Options = crate::NoFormatOptions;

        fn compile_defaults(
            &self,
            scope: &mut crate::RegistryBuildScope<'_>,
        ) -> Result<crate::CompiledOptions<Self::Options>, crate::OptionCompileError> {
            crate::NoFormatOptions::compile_defaults(scope)
        }

        fn descriptor(&self) -> FormatDescriptor {
            FormatDescriptor::builder(
                FormatId::new("dev.baozi.test.source-scope-allocation").unwrap(),
                "Source Scope Allocation",
            )
            .extensions(["scope-allocation"])
            .encoding(FormatEncoding::Binary)
            .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
            .build()
            .unwrap()
        }

        fn detect(
            &self,
            _probe: &crate::Probe<'_>,
            _hint: &ReadHint,
            _scope: &mut DetectionScope<'_, '_>,
        ) -> DetectionResult<DetectionOutcome> {
            Ok(DetectionOutcome::Match(ReadConfidence::Certain))
        }

        fn parse(
            &self,
            _options: &Self::Options,
            context: &mut crate::ParseContext<'_, '_>,
        ) -> ParseResult<()> {
            context.with_execution(|root| -> ParseResult<()> {
                let mut scope = root.services()?;
                let document = scope
                    .source()
                    .try_clone()
                    .map_err(|_| scope.allocation_failure("source-scope-document"))?;
                let mut builder = scope.source_scope_builder(
                    SourceScopeSelectionRequest::DocumentDefault,
                    SourceScopeSelectionOutcome::parentless_node_projection(document),
                )?;
                builder.observe(
                    &mut scope,
                    SourceScopeEntityKind::Scene,
                    SourceScopeDisposition::Selected,
                    "scene",
                )?;
                let finalized = builder.finish(&mut scope)?;
                inject_next_accounted_allocation_failure();
                scope.record_source_scope(finalized)?;
                unreachable!("the injected loss-evidence allocation must fail")
            })
        }
    }

    #[test]
    fn collapsed_bundle_allocation_failure_publishes_no_partial_evidence() {
        let mut registry = RegistryBuilder::new();
        registry
            .register(CollapsedAllocationFailureImporter)
            .unwrap();
        let snapshot = registry.freeze(RegistryBuildLimits::default()).unwrap();
        let limits = ResourceLimits {
            max_source_scope_detail_bytes: 0,
            ..ResourceLimits::default()
        };
        let config = ImportConfigOverlay::default()
            .with_resource_limits(ResourceLimitsOverlay::from_limits(limits))
            .with_loss_policy(
                LossPolicy::preserve_or_fail().allow(ImportLossKind::ProvenanceEvidenceLoss),
            );
        let request = snapshot
            .plan(
                &ImportRequest::from_layers(
                    ImportRequestLayer::default(),
                    ImportRequestLayer::new(config),
                ),
                OptionCompileLimits::default(),
            )
            .unwrap();

        let error = request
            .read_bytes(
                AssetPath::new("scene.scope-allocation").unwrap(),
                b"scope",
                ImportOperationControl::default(),
            )
            .unwrap_err();
        let failure = error
            .into_operation_failure()
            .expect("the allocation fault occurs after session creation");

        assert!(failure.source_scope_record().is_none());
        assert!(failure.loss_evidence().is_empty());
        assert!(failure.diagnostics().is_empty());
        for dimension in [
            ImportDimension::SourceScopeRecordCount,
            ImportDimension::LossEvidenceCount,
            ImportDimension::LossEvidenceBytes,
            ImportDimension::DiagnosticCount,
            ImportDimension::DiagnosticBytes,
        ] {
            assert_eq!(
                failure.ledger().dimension(dimension).unwrap().consumed(),
                0,
                "{dimension:?} must not claim an unpublished bundle"
            );
        }
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert!(matches!(
            failure.cause(),
            ImportCause::Scope(abort)
                if matches!(
                    abort.cause(),
                    ScopeAbortCause::Allocation {
                        resource: "loss-evidence-capacity"
                    }
                )
        ));
    }
}
