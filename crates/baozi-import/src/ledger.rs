use crate::limits::ResourceLimits;
use baozi_operation::{
    DimensionPolicy, LedgerCause, LedgerLimits, LedgerLimitsError, LedgerSnapshot, LiveReservation,
    OperationControl, OperationCoordinator, OperationOwner, PendingPublication, ProgressPhase,
    ProgressUnit, PublicationReservationBatch, PublicationReservationBatchError,
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
}

impl ImportDimension {
    const ALL: [Self; 48] = [
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
        }
    }
}

const _: () =
    assert!(ImportDimension::ALL.len() == ImportDimension::DecompressedBytes as usize + 1);

pub(crate) fn import_ledger_limits(
    limits: &ResourceLimits,
) -> Result<LedgerLimits<ImportDimension>, LedgerLimitsError<ImportDimension>> {
    LedgerLimits::new(
        limits.max_live_bytes,
        ImportDimension::ALL.map(|dimension| (dimension, dimension.policy(limits))),
    )
}

pub type ImportOperationControl = OperationControl<ImportProgressPhase, ImportProgressUnit>;
pub type ImportLedgerSnapshot = LedgerSnapshot<ImportLedgerPhase, ImportDimension>;

pub(crate) type ImportCoordinator = OperationCoordinator<
    ImportOperationOwner,
    ImportLedgerPhase,
    ImportProgressPhase,
    ImportProgressUnit,
    ImportDimension,
>;

pub(crate) fn tighten_selected_format_limits(
    coordinator: &mut ImportCoordinator,
    limits: &ResourceLimits,
) -> Result<(), LedgerCause<ImportDimension>> {
    for dimension in [
        ImportDimension::PrimaryBytes,
        ImportDimension::DecodedTextBytes,
        ImportDimension::LineIndexBytes,
        ImportDimension::Tokens,
        ImportDimension::CanonicalStringBytes,
    ] {
        coordinator.tighten_dimension_policy(dimension, dimension.policy(limits))?;
    }
    Ok(())
}

pub(crate) type ImportReservation =
    LiveReservation<ImportOperationOwner, ImportLedgerPhase, ImportDimension>;
pub(crate) type ImportPublicationBatch =
    PublicationReservationBatch<ImportOperationOwner, ImportLedgerPhase, ImportDimension>;
pub(crate) type ImportPublicationBatchError =
    PublicationReservationBatchError<ImportOperationOwner, ImportLedgerPhase, ImportDimension>;
pub(crate) type ImportPendingPublication =
    PendingPublication<ImportOperationOwner, ImportLedgerPhase, ImportDimension>;

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
