use crate::error::ScopeAbortCause;
use crate::ledger::ImportDimension;
use crate::metered_storage::AccountedVec;
use crate::scope::{
    ImportStorageClass, METERED_COPY_POLL_INTERVAL, ParseFailure, ParseResult, ParseScope,
};
use sha2::{Digest, Sha256};
use std::fmt;

const SOURCE_SCOPE_DIGEST_DOMAIN: &[u8] = b"baozi.source-scope.v1\0";

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
}

impl SourceScopeEntityKind {
    const COUNT: usize = Self::AnimationSampler as usize + 1;

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
    counts: [SourceScopeCounts; SourceScopeEntityKind::COUNT],
    digest: SourceScopeDigest,
    detail_state: SourceScopeDetailState,
    detail_indexes: Vec<SourceScopeDetailIndex>,
    detail_bytes: Vec<u8>,
    collapsed_detail_count: u64,
    collapsed_detail_bytes: u64,
}

impl SourceScopeRecord {
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
#[must_use = "finish and submit source-scope evidence or drop the builder"]
pub struct SourceScopeRecordBuilder<'scope, 'session> {
    scope: &'scope ParseScope<'session>,
    counts: [SourceScopeCounts; SourceScopeEntityKind::COUNT],
    digest: Sha256,
    detail_state: SourceScopeDetailState,
    detail_indexes: AccountedVec<SourceScopeDetailIndex>,
    detail_bytes: AccountedVec<u8>,
    observed_count: u64,
    observed_bytes: u64,
}

impl<'scope, 'session> SourceScopeRecordBuilder<'scope, 'session> {
    pub(crate) fn new(scope: &'scope ParseScope<'session>) -> Self {
        let mut digest = Sha256::new();
        digest.update(SOURCE_SCOPE_DIGEST_DOMAIN);
        Self {
            scope,
            counts: [SourceScopeCounts::default(); SourceScopeEntityKind::COUNT],
            digest,
            detail_state: SourceScopeDetailState::Detailed,
            detail_indexes: AccountedVec::new(),
            detail_bytes: AccountedVec::new(),
            observed_count: 0,
            observed_bytes: 0,
        }
    }

    pub fn observe(
        &mut self,
        kind: SourceScopeEntityKind,
        disposition: SourceScopeDisposition,
        identity: &str,
    ) -> ParseResult<()> {
        self.scope.poll()?;
        let identity_bytes = u64::try_from(identity.len()).map_err(|_| {
            self.scope
                .resource_limit_exceeded("source-scope-identity-bytes", u64::MAX)
        })?;
        self.scope.enforce_resource_limit(
            "source-scope-identity-bytes",
            identity_bytes,
            u64::try_from(self.scope.limits().max_string_bytes).unwrap_or(u64::MAX),
        )?;
        let observed_count = self.observed_count.checked_add(1).ok_or_else(|| {
            self.scope
                .resource_limit_exceeded("source-scope-items", u64::MAX)
        })?;
        let observed_bytes = self
            .observed_bytes
            .checked_add(identity_bytes)
            .ok_or_else(|| {
                self.scope
                    .resource_limit_exceeded("source-scope-digest-bytes", u64::MAX)
            })?;
        self.scope.source_scope_charge_batch(&[
            (ImportDimension::SourceScopeItemCount, 1),
            (ImportDimension::SourceScopeDigestBytes, identity_bytes),
        ])?;

        // This framing is the v1 digest wire: typed tag, disposition, LE length, then full bytes.
        self.digest.update([kind as u8, disposition as u8]);
        self.digest.update(identity_bytes.to_le_bytes());
        for (chunk_index, chunk) in identity
            .as_bytes()
            .chunks(METERED_COPY_POLL_INTERVAL)
            .enumerate()
        {
            if chunk_index != 0 {
                self.scope.poll()?;
            }
            self.digest.update(chunk);
        }

        self.counts[kind.index()]
            .increment(disposition)
            .map_err(|()| {
                self.scope
                    .resource_limit_exceeded("source-scope-typed-count", u64::MAX)
            })?;
        self.observed_count = observed_count;
        self.observed_bytes = observed_bytes;

        if self.detail_state == SourceScopeDetailState::Collapsed {
            return Ok(());
        }

        let exceeds_count = observed_count > self.scope.limits().max_source_scope_detail_count;
        let exceeds_bytes = observed_bytes > self.scope.limits().max_source_scope_detail_bytes;
        if exceeds_count || exceeds_bytes {
            if !self
                .scope
                .loss_policy()
                .allows(crate::ImportLossKind::ProvenanceEvidenceLoss)
            {
                let (resource, limit) = if exceeds_count {
                    (
                        "source-scope-detail-count",
                        self.scope.limits().max_source_scope_detail_count,
                    )
                } else {
                    (
                        "source-scope-detail-bytes",
                        self.scope.limits().max_source_scope_detail_bytes,
                    )
                };
                return Err(self.scope.resource_limit_exceeded(resource, limit));
            }
            self.detail_state = SourceScopeDetailState::Collapsed;
            self.detail_indexes = AccountedVec::new();
            self.detail_bytes = AccountedVec::new();
            return Ok(());
        }

        self.scope.source_scope_charge_batch(&[
            (ImportDimension::SourceScopeDetailCount, 1),
            (ImportDimension::SourceScopeDetailBytes, identity_bytes),
        ])?;

        self.detail_indexes
            .try_reserve(1, "source-scope-detail-index", |bytes| {
                self.scope
                    .reserve_storage_cause(ImportStorageClass::FormatNormalization, bytes)
            })
            .map_err(|cause| self.scope.source_scope_abort(cause))?;
        self.detail_bytes
            .try_reserve(identity.len(), "source-scope-detail-bytes", |bytes| {
                self.scope
                    .reserve_storage_cause(ImportStorageClass::FormatNormalization, bytes)
            })
            .map_err(|cause| self.scope.source_scope_abort(cause))?;

        let start = self.detail_bytes.len();
        for chunk in identity.as_bytes().chunks(METERED_COPY_POLL_INTERVAL) {
            self.scope.poll()?;
            self.detail_bytes
                .try_extend_from_slice_reserved(chunk)
                .map_err(|()| {
                    self.scope.source_scope_descriptor_failure(
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
                self.scope
                    .source_scope_descriptor_failure("source-scope-index-capacity-not-admitted")
            })?;
        Ok(())
    }

    pub fn finish(self) -> ParseResult<FinalizedSourceScopeRecord<'scope, 'session>> {
        self.scope.source_scope_preflight()?;
        let digest = SourceScopeDigest(self.digest.finalize().into());
        Ok(FinalizedSourceScopeRecord {
            scope: self.scope,
            record: LiveSourceScopeRecord {
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
            },
        })
    }
}

/// A finalized record that can only be submitted back to its originating ParseScope.
#[must_use = "submit finalized source-scope evidence to its originating parse scope"]
pub struct FinalizedSourceScopeRecord<'scope, 'session> {
    scope: &'scope ParseScope<'session>,
    record: LiveSourceScopeRecord,
}

impl FinalizedSourceScopeRecord<'_, '_> {
    pub(crate) fn belongs_to(&self, scope: &ParseScope<'_>) -> bool {
        std::ptr::eq(self.scope, scope)
    }

    pub(crate) fn into_record(self) -> LiveSourceScopeRecord {
        self.record
    }
}

pub(crate) struct LiveSourceScopeRecord {
    counts: [SourceScopeCounts; SourceScopeEntityKind::COUNT],
    digest: SourceScopeDigest,
    detail_state: SourceScopeDetailState,
    detail_indexes: AccountedVec<SourceScopeDetailIndex>,
    detail_bytes: AccountedVec<u8>,
    collapsed_detail_count: u64,
    collapsed_detail_bytes: u64,
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
        let detail_indexes = self.detail_indexes.into_terminal().into_values();
        let detail_bytes = self.detail_bytes.into_terminal().into_values();
        SourceScopeRecord {
            counts: self.counts,
            digest: self.digest,
            detail_state: self.detail_state,
            detail_indexes,
            detail_bytes,
            collapsed_detail_count: self.collapsed_detail_count,
            collapsed_detail_bytes: self.collapsed_detail_bytes,
        }
    }
}

impl fmt::Debug for FinalizedSourceScopeRecord<'_, '_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FinalizedSourceScopeRecord")
            .finish_non_exhaustive()
    }
}

impl ParseScope<'_> {
    pub(crate) fn source_scope_descriptor_failure(&self, code: &'static str) -> ParseFailure {
        self.source_scope_abort(ScopeAbortCause::DescriptorContract { code })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SceneDraft as CanonicalSceneDraft;
    use crate::metered_storage::inject_next_accounted_allocation_failure;
    use crate::{
        CapabilityStatus, DetectionOutcome, DetectionResult, DetectionScope, FormatCapability,
        FormatDescriptor, FormatEncoding, FormatImporter, ImportCause, ImportConfigOverlay,
        ImportLossKind, ImportOperationControl, ImportRequest, ImportRequestLayer, LossPolicy,
        OptionCompileLimits, ReadConfidence, ReadHint, RegistryBuildLimits, RegistryBuilder,
        ResourceLimits, ResourceLimitsOverlay,
    };
    use baozi_core::FormatId;
    use baozi_io::AssetPath;

    struct CollapsedAllocationFailureImporter;

    impl FormatImporter for CollapsedAllocationFailureImporter {
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
            _scope: &DetectionScope<'_>,
        ) -> DetectionResult<DetectionOutcome> {
            Ok(DetectionOutcome::Match(ReadConfidence::Certain))
        }

        fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<CanonicalSceneDraft> {
            let mut builder = scope.source_scope_builder()?;
            builder.observe(
                SourceScopeEntityKind::Scene,
                SourceScopeDisposition::Selected,
                "scene",
            )?;
            let finalized = builder.finish()?;
            inject_next_accounted_allocation_failure();
            scope.record_source_scope(finalized)?;
            unreachable!("the injected loss-evidence allocation must fail")
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
            .compile_request(
                &ImportRequest::from_layers(
                    ImportRequestLayer::default(),
                    ImportRequestLayer::new(config),
                ),
                OptionCompileLimits::default(),
            )
            .unwrap();

        let error = crate::import_bytes(
            &request,
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
