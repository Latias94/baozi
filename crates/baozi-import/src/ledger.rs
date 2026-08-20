use crate::limits::ResourceLimits;
use crate::store::{
    ImportStoreHost, ImportStoreHostError, ImportStoreTerminalCause, ImportStoreTerminalEvidence,
    ImportStoreTerminalFailureClass,
};
use baozi_operation::{
    DimensionPolicy, LedgerLimits, LedgerLimitsError, OperationControl, OperationOwner,
    ProgressPhase, ProgressUnit,
};

#[derive(Debug)]
pub(crate) struct ImportOperationOwner;

impl OperationOwner for ImportOperationOwner {}

/// Accounting attribution. It is not lifecycle evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ImportLedgerPhase {
    PrimarySnapshot,
    Detection,
    DependencyAcquisition,
    BackendDecode,
    FormatNormalization,
    SceneConstruction,
    Validation,
    OutcomeFinalization,
}

impl ImportLedgerPhase {
    /// Complete canonical accounting-phase vocabulary for one import operation.
    pub const ALL: [Self; 8] = [
        Self::PrimarySnapshot,
        Self::Detection,
        Self::DependencyAcquisition,
        Self::BackendDecode,
        Self::FormatNormalization,
        Self::SceneConstruction,
        Self::Validation,
        Self::OutcomeFinalization,
    ];
}

/// Transient observer vocabulary. It cannot advance lifecycle evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ImportProgressPhase {
    AcquirePrimary,
    Detect,
    AcquireDependency,
    Decode,
    Normalize,
    BuildDraft,
    Validate,
    Finalize,
}

impl ProgressPhase for ImportProgressPhase {
    fn order(self) -> u32 {
        self as u32
    }

    fn stable_id(self) -> &'static str {
        match self {
            Self::AcquirePrimary => "import.acquire_primary",
            Self::Detect => "import.detect",
            Self::AcquireDependency => "import.acquire_dependency",
            Self::Decode => "import.decode",
            Self::Normalize => "import.normalize",
            Self::BuildDraft => "import.build_draft",
            Self::Validate => "import.validate",
            Self::Finalize => "import.finalize",
        }
    }
}

/// Stable terminal failure classes projected from the unique retained-store ledger header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportTerminalFailureClass {
    Requested,
    Control,
    ProductGraph,
    Ledger,
}

/// Import-owned terminal failure identity with no retained-store key or implementation detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportTerminalFailure {
    class: ImportTerminalFailureClass,
    code: u32,
}

impl ImportTerminalFailure {
    const fn from_store(class: ImportStoreTerminalFailureClass, code: u32) -> Self {
        let class = match class {
            ImportStoreTerminalFailureClass::Requested => ImportTerminalFailureClass::Requested,
            ImportStoreTerminalFailureClass::Control => ImportTerminalFailureClass::Control,
            ImportStoreTerminalFailureClass::ProductGraph => {
                ImportTerminalFailureClass::ProductGraph
            }
            ImportStoreTerminalFailureClass::Ledger => ImportTerminalFailureClass::Ledger,
        };
        Self { class, code }
    }

    pub const fn class(self) -> ImportTerminalFailureClass {
        self.class
    }

    pub const fn code(self) -> u32 {
        self.code
    }
}

/// Terminal cause projected once from the sole import ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportTerminalCause {
    Success,
    Failure(ImportTerminalFailure),
}

impl ImportTerminalCause {
    pub(crate) const fn from_store(cause: ImportStoreTerminalCause) -> Self {
        match cause {
            ImportStoreTerminalCause::Success => Self::Success,
            ImportStoreTerminalCause::Failure { class, code } => {
                Self::Failure(ImportTerminalFailure::from_store(class, code))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ImportProgressUnit {
    Bytes,
    Candidates,
    Dependencies,
    Records,
    GraphItems,
    CanonicalItems,
    WorkUnits,
}

impl ProgressUnit for ImportProgressUnit {
    fn stable_id(self) -> &'static str {
        match self {
            Self::Bytes => "bytes",
            Self::Candidates => "candidates",
            Self::Dependencies => "dependencies",
            Self::Records => "records",
            Self::GraphItems => "graph_items",
            Self::CanonicalItems => "canonical_items",
            Self::WorkUnits => "work_units",
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ImportDimension {
    AssetOpen,
    DependencyReference,
    PrimaryBytes,
    DependencyBytes,
    TotalAssetBytes,
    DataUriBytes,
    DiagnosticCount,
    DiagnosticBytes,
    DecodedTextBytes,
    LineIndexBytes,
    RecoveryCount,
    RecoveryRawBytes,
    LossEvidenceCount,
    LossEvidenceBytes,
    SourceScopeRecordCount,
    SourceScopeItemCount,
    SourceScopeDetailCount,
    SourceScopeDetailBytes,
    SourceScopeDigestBytes,
    DetectionWork,
    WorkUnits,
    Tokens,
    Records,
    Meshes,
    Vertices,
    Faces,
    GraphNodes,
    GraphEdges,
    GraphDepth,
    GraphWork,
    ResourceGraphNodes,
    ResourceGraphRelations,
    ResourceGraphWork,
    InputManifestBytes,
    InputManifestWork,
    Primitives,
    AttributeStreams,
    Solids,
    DecodeWork,
    ScratchBytes,
    CanonicalNodes,
    CanonicalPrimitives,
    AttributeCells,
    TopologyCells,
    InfluenceCells,
    MorphCells,
    AnimationCells,
    CanonicalStringBytes,
    CanonicalPayloadBytes,
    CanonicalOutputBytes,
    ArchiveBytes,
    CompressedBytes,
    DecompressedBytes,
    NegativeProofCanonicalBytes,
    NegativeProofTemporaryBytes,
    NegativeProofConstructionWork,
    NegativeProofVerificationWork,
    DependencyAcquisitionWork,
}

impl ImportDimension {
    pub(crate) const ALL: [Self; 58] = [
        Self::AssetOpen,
        Self::DependencyReference,
        Self::PrimaryBytes,
        Self::DependencyBytes,
        Self::TotalAssetBytes,
        Self::DataUriBytes,
        Self::DiagnosticCount,
        Self::DiagnosticBytes,
        Self::DecodedTextBytes,
        Self::LineIndexBytes,
        Self::RecoveryCount,
        Self::RecoveryRawBytes,
        Self::LossEvidenceCount,
        Self::LossEvidenceBytes,
        Self::SourceScopeRecordCount,
        Self::SourceScopeItemCount,
        Self::SourceScopeDetailCount,
        Self::SourceScopeDetailBytes,
        Self::SourceScopeDigestBytes,
        Self::DetectionWork,
        Self::WorkUnits,
        Self::Tokens,
        Self::Records,
        Self::Meshes,
        Self::Vertices,
        Self::Faces,
        Self::GraphNodes,
        Self::GraphEdges,
        Self::GraphDepth,
        Self::GraphWork,
        Self::ResourceGraphNodes,
        Self::ResourceGraphRelations,
        Self::ResourceGraphWork,
        Self::InputManifestBytes,
        Self::InputManifestWork,
        Self::Primitives,
        Self::AttributeStreams,
        Self::Solids,
        Self::DecodeWork,
        Self::ScratchBytes,
        Self::CanonicalNodes,
        Self::CanonicalPrimitives,
        Self::AttributeCells,
        Self::TopologyCells,
        Self::InfluenceCells,
        Self::MorphCells,
        Self::AnimationCells,
        Self::CanonicalStringBytes,
        Self::CanonicalPayloadBytes,
        Self::CanonicalOutputBytes,
        Self::ArchiveBytes,
        Self::CompressedBytes,
        Self::DecompressedBytes,
        Self::NegativeProofCanonicalBytes,
        Self::NegativeProofTemporaryBytes,
        Self::NegativeProofConstructionWork,
        Self::NegativeProofVerificationWork,
        Self::DependencyAcquisitionWork,
    ];

    fn policy(self, limits: &ResourceLimits) -> DimensionPolicy {
        let counter = |limit| DimensionPolicy::counter(limit, limit, 0);
        let committed_counter = |limit| DimensionPolicy::counter(limit, limit, limit);
        let bytes = |limit| DimensionPolicy::reservable_bytes(limit, limit, 0, limit);
        let committed_bytes = |limit| DimensionPolicy::reservable_bytes(limit, limit, limit, limit);
        let count = |value: usize| u64::try_from(value).unwrap_or(u64::MAX);

        match self {
            Self::AssetOpen => counter(limits.max_open_assets),
            Self::DependencyReference => counter(limits.max_dependency_references),
            Self::DependencyAcquisitionWork => DimensionPolicy::counter(
                limits
                    .max_dependency_references
                    .max(limits.max_dependency_acquisition_work),
                limits.max_dependency_acquisition_work,
                0,
            ),
            Self::PrimaryBytes => bytes(limits.max_primary_asset_bytes),
            // The per-dependency bound is enforced locally. This dimension is aggregate.
            Self::DependencyBytes => bytes(limits.max_total_asset_bytes),
            Self::TotalAssetBytes => counter(limits.max_total_asset_bytes),
            Self::DataUriBytes => bytes(limits.max_data_uri_bytes),
            Self::DiagnosticCount => DimensionPolicy::counter(
                limits.max_attempted_diagnostics,
                limits.max_diagnostics.min(limits.max_attempted_diagnostics),
                0,
            ),
            Self::DiagnosticBytes => DimensionPolicy::reservable_bytes(
                limits.max_attempted_diagnostic_bytes,
                limits
                    .max_diagnostic_bytes
                    .min(limits.max_attempted_diagnostic_bytes),
                0,
                limits
                    .max_diagnostic_bytes
                    .min(limits.max_attempted_diagnostic_bytes),
            ),
            Self::DecodedTextBytes => bytes(limits.max_decoded_utf8_bytes),
            Self::LineIndexBytes => bytes(limits.max_line_index_bytes),
            Self::RecoveryCount => counter(limits.max_attempted_recoveries),
            Self::RecoveryRawBytes => counter(limits.max_attempted_recovery_bytes),
            Self::LossEvidenceCount => counter(limits.max_loss_evidence),
            Self::LossEvidenceBytes => bytes(limits.max_loss_evidence_bytes),
            Self::SourceScopeRecordCount => counter(limits.max_source_scope_records),
            Self::SourceScopeItemCount => counter(limits.max_source_scope_items),
            Self::SourceScopeDetailCount => counter(limits.max_source_scope_detail_count),
            Self::SourceScopeDetailBytes => counter(limits.max_source_scope_detail_bytes),
            Self::SourceScopeDigestBytes => counter(limits.max_source_scope_digest_bytes),
            Self::DetectionWork => counter(limits.max_detection_work),
            Self::WorkUnits => counter(limits.max_work_units),
            Self::Tokens => counter(limits.max_tokens),
            Self::Records => counter(count(limits.max_records)),
            Self::Meshes => counter(count(limits.max_meshes)),
            Self::Vertices => counter(count(limits.max_vertices)),
            Self::Faces => counter(count(limits.max_faces)),
            Self::GraphNodes => counter(count(limits.max_graph_nodes)),
            Self::GraphEdges => counter(count(limits.max_graph_edges)),
            Self::GraphDepth => counter(count(limits.max_graph_depth)),
            Self::GraphWork => counter(count(limits.max_graph_work)),
            Self::ResourceGraphNodes => counter(count(limits.max_resource_graph_nodes)),
            Self::ResourceGraphRelations => counter(count(limits.max_resource_graph_relations)),
            Self::ResourceGraphWork => counter(limits.max_resource_graph_work),
            Self::InputManifestBytes => bytes(limits.max_input_manifest_bytes),
            Self::InputManifestWork => counter(limits.max_input_manifest_work),
            Self::Primitives => counter(count(limits.max_primitives)),
            Self::AttributeStreams => counter(count(limits.max_total_vertex_attribute_streams)),
            Self::Solids => counter(count(limits.max_solids)),
            Self::DecodeWork => counter(limits.max_decode_work),
            Self::ScratchBytes => bytes(limits.max_live_bytes),
            Self::CanonicalNodes => committed_counter(count(limits.max_graph_nodes)),
            Self::CanonicalPrimitives => committed_counter(count(limits.max_primitives)),
            Self::AttributeCells => {
                committed_counter(count(limits.max_total_vertex_attribute_cells))
            }
            Self::TopologyCells => committed_counter(count(limits.max_total_topology_cells)),
            Self::InfluenceCells => committed_counter(count(limits.max_influence_cells)),
            Self::MorphCells => committed_counter(count(limits.max_morph_cells)),
            Self::AnimationCells => committed_counter(count(limits.max_animation_cells)),
            Self::CanonicalStringBytes => committed_bytes(count(limits.max_canonical_string_bytes)),
            Self::CanonicalPayloadBytes | Self::CanonicalOutputBytes => {
                committed_bytes(limits.max_canonical_payload_bytes)
            }
            Self::ArchiveBytes | Self::CompressedBytes | Self::DecompressedBytes => {
                bytes(limits.max_total_asset_bytes)
            }
            Self::NegativeProofCanonicalBytes => counter(limits.max_negative_proof_canonical_bytes),
            Self::NegativeProofTemporaryBytes => bytes(limits.max_negative_proof_temporary_bytes),
            Self::NegativeProofConstructionWork => {
                counter(limits.max_negative_proof_construction_work)
            }
            Self::NegativeProofVerificationWork => {
                counter(limits.max_negative_proof_verification_work)
            }
        }
    }
}

const _: () =
    assert!(ImportDimension::ALL.len() == ImportDimension::DependencyAcquisitionWork as usize + 1);

pub(crate) fn import_ledger_limits(
    limits: &ResourceLimits,
) -> Result<LedgerLimits<ImportDimension>, LedgerLimitsError<ImportDimension>> {
    LedgerLimits::new(
        limits.max_live_bytes,
        ImportDimension::ALL.map(|dimension| (dimension, dimension.policy(limits))),
    )
}

pub type ImportOperationControl = OperationControl<ImportProgressPhase, ImportProgressUnit>;
/// Import-owned immutable projection of the unique retained-store terminal evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportLedgerSnapshot {
    evidence: ImportStoreTerminalEvidence,
}

impl ImportLedgerSnapshot {
    pub(crate) const fn from_store(evidence: ImportStoreTerminalEvidence) -> Self {
        Self { evidence }
    }

    pub const fn current_live_bytes(&self) -> u64 {
        self.evidence.current_live_bytes()
    }

    pub const fn peak_live_bytes(&self) -> u64 {
        self.evidence.peak_live_bytes()
    }

    pub const fn active_reservations(&self) -> u64 {
        self.evidence.active_reservations()
    }

    /// Returns the terminal operation's currently retained evidence bytes.
    ///
    /// A fully published terminal must report zero. The value is an import-owned scalar summary;
    /// no retained-store evidence or key escapes through this accessor.
    pub const fn current_evidence_bytes(&self) -> u64 {
        self.evidence.current_evidence_bytes()
    }

    /// Returns the peak evidence bytes retained while producing this terminal.
    pub const fn peak_evidence_bytes(&self) -> u64 {
        self.evidence.peak_evidence_bytes()
    }

    /// Returns the sole terminal cause frozen in this import ledger.
    pub const fn terminal_cause(&self) -> ImportTerminalCause {
        ImportTerminalCause::from_store(self.evidence.cause())
    }

    /// Returns the logical canonical members committed by the successful joint publication.
    ///
    /// This is terminal attribution, not physical byte ownership. The producer-visible
    /// [`ImportDimension::CanonicalOutputBytes`] row therefore remains a live-storage view and
    /// does not duplicate this committed destination count.
    pub const fn committed_canonical_members(&self) -> u64 {
        self.evidence.committed_canonical_members()
    }

    pub fn dimension(&self, dimension: ImportDimension) -> Option<ImportDimensionSnapshot> {
        self.evidence
            .dimension(dimension)
            .map(ImportDimensionSnapshot::from_store)
    }

    pub const fn last_completed_phase(&self) -> Option<ImportLedgerPhase> {
        self.evidence.last_completed_phase()
    }
}

/// Import-owned accounting evidence for one producer-visible dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportDimensionSnapshot {
    attempted: u64,
    consumed: u64,
    current_live_bytes: u64,
    peak_live_bytes: u64,
    committed: u64,
    published_outcome_evidence: u64,
}

impl ImportDimensionSnapshot {
    const fn from_store(evidence: crate::store::ImportStoreDimensionEvidence) -> Self {
        Self {
            attempted: evidence.attempted(),
            consumed: evidence.consumed(),
            current_live_bytes: evidence.current_live_bytes(),
            peak_live_bytes: evidence.peak_live_bytes(),
            committed: evidence.committed(),
            published_outcome_evidence: evidence.published_outcome_evidence(),
        }
    }

    pub const fn attempted(self) -> u64 {
        self.attempted
    }

    pub const fn consumed(self) -> u64 {
        self.consumed
    }

    pub const fn current_live_bytes(self) -> u64 {
        self.current_live_bytes
    }

    pub const fn peak_live_bytes(self) -> u64 {
        self.peak_live_bytes
    }

    pub const fn committed(self) -> u64 {
        self.committed
    }

    pub const fn published_outcome_evidence(self) -> u64 {
        self.published_outcome_evidence
    }
}

pub(crate) fn tighten_selected_format_limits(
    host: ImportStoreHost<'_>,
    limits: &ResourceLimits,
) -> Result<(), ImportStoreHostError> {
    for dimension in [
        ImportDimension::PrimaryBytes,
        ImportDimension::DecodedTextBytes,
        ImportDimension::LineIndexBytes,
        ImportDimension::Tokens,
        ImportDimension::CanonicalStringBytes,
    ] {
        host.tighten_dimension_policy(dimension, dimension.policy(limits))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::TypeId;

    #[test]
    fn import_ledger_phase_catalog_is_complete_unique_and_canonical() {
        let expected = [
            ImportLedgerPhase::PrimarySnapshot,
            ImportLedgerPhase::Detection,
            ImportLedgerPhase::DependencyAcquisition,
            ImportLedgerPhase::BackendDecode,
            ImportLedgerPhase::FormatNormalization,
            ImportLedgerPhase::SceneConstruction,
            ImportLedgerPhase::Validation,
            ImportLedgerPhase::OutcomeFinalization,
        ];

        assert_eq!(ImportLedgerPhase::ALL, expected);
        for (index, phase) in ImportLedgerPhase::ALL.iter().enumerate() {
            assert!(
                !ImportLedgerPhase::ALL[..index].contains(phase),
                "import ledger phase catalog contains a duplicate: {phase:?}"
            );
        }
        assert!(
            ImportLedgerPhase::ALL
                .windows(2)
                .all(|pair| pair[0] < pair[1]),
            "import ledger phase catalog must follow canonical Ord order"
        );
    }

    #[test]
    fn import_ledger_and_progress_phase_vocabularies_are_distinct_types() {
        assert_ne!(
            TypeId::of::<ImportLedgerPhase>(),
            TypeId::of::<ImportProgressPhase>()
        );
    }

    #[test]
    fn diagnostic_policy_never_relaxes_attempted_limits() {
        let limits = ResourceLimits {
            max_attempted_diagnostics: 2,
            max_diagnostics: 7,
            max_attempted_diagnostic_bytes: 11,
            max_diagnostic_bytes: 19,
            ..ResourceLimits::default()
        };

        let ledger = import_ledger_limits(&limits).expect("valid import ledger limits");
        let count = ledger
            .dimension(ImportDimension::DiagnosticCount)
            .expect("diagnostic count is exhaustive");
        let bytes = ledger
            .dimension(ImportDimension::DiagnosticBytes)
            .expect("diagnostic bytes are exhaustive");

        assert_eq!(count.attempted_limit(), 2);
        assert_eq!(count.consumed_limit(), 2);
        assert_eq!(bytes.attempted_limit(), 11);
        assert_eq!(bytes.consumed_limit(), 11);
        assert_eq!(bytes.live_byte_limit(), Some(11));
    }
}
