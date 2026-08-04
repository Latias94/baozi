use std::fmt::Write as _;

use allocation_counter::AllocationInfo;
use baozi_operation::__u4c::MeasurementReport;
use baozi_operation::{
    PhysicalStorageAccountFactorySnapshot, PhysicalStorageAccountSnapshot, StoreLedgerEvidence,
    StorePreparationFailureClass, StoreTerminalCause,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::super::super::construction::node_scene::snapshot::NodeSceneSnapshotTotals;
use super::super::super::construction::{FAILURE_TERMINAL_WORK_DIMENSION, STORE_DIMENSION};

pub(super) const REPORT_PREFIX: &str = "BAOZI_U4C_PRODUCTION_V2=";
const REPORT_SCHEMA_ID: &str = "baozi.u4c.production-report";
const REPORT_SCHEMA_VERSION: u32 = 2;
const REPORT_REVISION: u32 = 1;
const SCENARIO_SET_ID: &str = "canonical-store-production-v2";

#[derive(Serialize)]
pub(super) struct ProductionReport {
    schema_id: &'static str,
    schema_version: u32,
    report_revision: u32,
    scenario_set_id: &'static str,
    scale_scenarios: Vec<ScaleScenarioReport>,
    inherited_hot_leaf_scenario: InheritedHotLeafReport,
    obj_capability: ObjCapabilityReport,
    gltf_capability: GltfCapabilityReport,
    validation_faults: Vec<ValidationFaultReport>,
    protected_terminal_activation: ProtectedTerminalActivationReport,
    minimal_fallback: MinimalFallbackReport,
}

impl ProductionReport {
    pub(super) fn new(
        scale_scenarios: Vec<ScaleScenarioReport>,
        inherited_hot_leaf_scenario: InheritedHotLeafReport,
        obj_capability: ObjCapabilityReport,
        gltf_capability: GltfCapabilityReport,
        validation_faults: Vec<ValidationFaultReport>,
        protected_terminal_activation: ProtectedTerminalActivationReport,
        minimal_fallback: MinimalFallbackReport,
    ) -> Self {
        Self {
            schema_id: REPORT_SCHEMA_ID,
            schema_version: REPORT_SCHEMA_VERSION,
            report_revision: REPORT_REVISION,
            scenario_set_id: SCENARIO_SET_ID,
            scale_scenarios,
            inherited_hot_leaf_scenario,
            obj_capability,
            gltf_capability,
            validation_faults,
            protected_terminal_activation,
            minimal_fallback,
        }
    }
}

#[derive(Serialize)]
pub(super) struct ScaleScenarioReport {
    pub(super) scenario_id: &'static str,
    pub(super) image_count: usize,
    pub(super) canonical_block_count: usize,
    pub(super) foreign_canonical_block_count: usize,
    pub(super) target_generation_reservations: u64,
    pub(super) foreign_generation_reservations: u64,
    pub(super) typed_inspection_succeeded: bool,
    pub(super) foreign_directory_rejected: bool,
    pub(super) semantic_transcript: SemanticTranscriptReport,
    pub(super) evidence: EvidenceReport,
    pub(super) construction: PhaseReport,
    pub(super) typed_inspection: PhaseReport,
    pub(super) target_release: ReleaseReport,
    pub(super) foreign_release: ReleaseReport,
    pub(super) physical_account: ScalePhysicalReport,
    pub(super) factory_after_account_release: PhysicalFactoryReport,
}

#[derive(Serialize)]
pub(super) struct InheritedHotLeafReport {
    pub(super) scenario_id: &'static str,
    pub(super) generation_reservations: u64,
    pub(super) fresh: ShareGenerationReport,
    pub(super) derived: ShareGenerationReport,
    pub(super) fresh_hot_leaf: HotLeafObservationReport,
    pub(super) derived_hot_leaf: HotLeafObservationReport,
    pub(super) derived_after_parent_release_hot_leaf: HotLeafObservationReport,
    pub(super) fresh_report_release: ReleaseReport,
    pub(super) derived_report_release: ReleaseReport,
    pub(super) parent_release: ReleaseReport,
    pub(super) derived_release: ReleaseReport,
    pub(super) physical_account: SharePhysicalReport,
    pub(super) factory_after_account_release: PhysicalFactoryReport,
}

#[derive(Serialize)]
pub(super) struct ObjCapabilityReport {
    pub(super) entry_point: &'static str,
    pub(super) scenario_id: &'static str,
    pub(super) source_shape: ObjSourceShapeReport,
    pub(super) canonical_shape: ObjCanonicalShapeReport,
    pub(super) source_vector: FixedVectorReport,
    pub(super) equivalent_source_vector: FixedVectorReport,
    pub(super) equivalent_snapshot_digest_hex: String,
    pub(super) mutations: Vec<CapabilityMutationReport>,
    pub(super) generation_reservations: u64,
    pub(super) canonical_block_count: usize,
    pub(super) outcome_evidence_block_count: usize,
    pub(super) full_snapshot_digest_hex: String,
    pub(super) bounded_snapshot_digest_hex: String,
    pub(super) snapshot_totals: CanonicalSnapshotTotalsReport,
    pub(super) bounded_preview: CapabilityBoundedPreviewReport,
    pub(super) evidence: EvidenceReport,
    pub(super) construction: PhaseReport,
    pub(super) inspection: PhaseReport,
    pub(super) snapshot: PhaseReport,
    pub(super) release: ReleaseReport,
    pub(super) physical_account: CapabilityPhysicalReport,
    pub(super) factory_after_account_release: PhysicalFactoryReport,
}

#[derive(Serialize)]
pub(super) struct ObjSourceShapeReport {
    pub(super) position_count: usize,
    pub(super) texcoord_count: usize,
    pub(super) normal_count: usize,
    pub(super) face_count: usize,
    pub(super) corner_count: usize,
}

#[derive(Serialize)]
pub(super) struct ObjCanonicalShapeReport {
    pub(super) topology: &'static str,
    pub(super) vertex_count: u32,
    pub(super) corner_count: u32,
    pub(super) face_count: u32,
    pub(super) vertex_position_elements: u32,
    pub(super) corner_normal_elements: u32,
    pub(super) corner_texcoord_elements: u32,
    pub(super) face_smoothing_group_elements: u32,
}

#[derive(Serialize)]
pub(super) struct FixedVectorReport {
    pub(super) schema_id: &'static str,
    pub(super) observed_bytes: usize,
    pub(super) sha256: String,
}

#[derive(Serialize)]
pub(super) struct CapabilityMutationReport {
    pub(super) field_class: &'static str,
    pub(super) source_vector: FixedVectorReport,
    pub(super) snapshot_digest_hex: String,
}

#[derive(Serialize)]
pub(super) struct CanonicalSnapshotTotalsReport {
    pub(super) node_count: usize,
    pub(super) root_count: usize,
    pub(super) edge_count: usize,
    pub(super) camera_count: usize,
    pub(super) light_count: usize,
    pub(super) camera_attachment_count: usize,
    pub(super) light_attachment_count: usize,
    pub(super) image_count: usize,
    pub(super) sampler_count: usize,
    pub(super) texture_count: usize,
    pub(super) material_count: usize,
    pub(super) material_binding_count: usize,
    pub(super) image_payload_bytes: u64,
    pub(super) media_type_bytes: u64,
    pub(super) source_key_bytes: u64,
    pub(super) binding_semantic_scalar_count: u64,
    pub(super) material_factor_scalar_count: u64,
    pub(super) logical_mesh_count: usize,
    pub(super) primitive_count: usize,
    pub(super) attribute_count: usize,
    pub(super) mesh_instance_count: usize,
    pub(super) polygon_boundary_count: u64,
    pub(super) primitive_index_count: u64,
    pub(super) sparse_index_count: u64,
    pub(super) canonical_scalar_count: u64,
    pub(super) f32_scalar_count: u64,
    pub(super) u32_scalar_count: u64,
    pub(super) skin_count: usize,
    pub(super) logical_morph_count: usize,
    pub(super) morph_definition_count: usize,
    pub(super) primitive_deformation_count: usize,
    pub(super) morph_target_count: usize,
    pub(super) morph_delta_count: usize,
    pub(super) deformation_mesh_instance_count: usize,
    pub(super) skin_joint_count: u64,
    pub(super) inverse_bind_matrix_count: u64,
    pub(super) skin_correction_matrix_count: u64,
    pub(super) skin_name_bytes: u64,
    pub(super) morph_default_weight_count: u64,
    pub(super) morph_name_bytes: u64,
    pub(super) influence_row_count: u64,
    pub(super) influence_count: u64,
    pub(super) morph_value_vector_count: u64,
    pub(super) morph_sparse_index_count: u64,
    pub(super) instance_morph_weight_count: u64,
    pub(super) animation_clip_count: usize,
    pub(super) animation_channel_count: usize,
    pub(super) animation_time_scalar_count: u64,
    pub(super) animation_value_scalar_count: u64,
    pub(super) animation_in_tangent_scalar_count: u64,
    pub(super) animation_out_tangent_scalar_count: u64,
    pub(super) profile_resource_count: usize,
    pub(super) profile_source_count: usize,
    pub(super) profile_provenance_node_count: usize,
    pub(super) profile_operation_count: usize,
    pub(super) profile_evidence_count: usize,
    pub(super) profile_owner_count: usize,
    pub(super) profile_attachment_count: usize,
    pub(super) profile_metadata_entry_count: usize,
    pub(super) profile_parent_reference_count: u64,
    pub(super) profile_wide_value_count: u64,
    pub(super) profile_lineage_text_bytes: u64,
    pub(super) profile_lineage_digest_bytes: u64,
    pub(super) profile_metadata_text_bytes: u64,
    pub(super) profile_metadata_payload_bytes: u64,
    pub(super) profile_metadata_bool_scalar_count: u64,
    pub(super) profile_metadata_i64_scalar_count: u64,
    pub(super) profile_metadata_u64_scalar_count: u64,
    pub(super) profile_metadata_f64_scalar_count: u64,
    pub(super) profile_metadata_f32_scalar_count: u64,
    pub(super) transcript_bytes: u64,
    pub(super) work_units: u64,
}

impl From<NodeSceneSnapshotTotals> for CanonicalSnapshotTotalsReport {
    fn from(totals: NodeSceneSnapshotTotals) -> Self {
        let NodeSceneSnapshotTotals {
            node_count,
            root_count,
            edge_count,
            camera_count,
            light_count,
            camera_attachment_count,
            light_attachment_count,
            image_count,
            sampler_count,
            texture_count,
            material_count,
            material_binding_count,
            image_payload_bytes,
            media_type_bytes,
            source_key_bytes,
            binding_semantic_scalar_count,
            material_factor_scalar_count,
            logical_mesh_count,
            primitive_count,
            attribute_count,
            mesh_instance_count,
            polygon_boundary_count,
            primitive_index_count,
            sparse_index_count,
            canonical_scalar_count,
            f32_scalar_count,
            u32_scalar_count,
            skin_count,
            logical_morph_count,
            morph_definition_count,
            primitive_deformation_count,
            morph_target_count,
            morph_delta_count,
            deformation_mesh_instance_count,
            skin_joint_count,
            inverse_bind_matrix_count,
            skin_correction_matrix_count,
            skin_name_bytes,
            morph_default_weight_count,
            morph_name_bytes,
            influence_row_count,
            influence_count,
            morph_value_vector_count,
            morph_sparse_index_count,
            instance_morph_weight_count,
            animation_clip_count,
            animation_channel_count,
            animation_time_scalar_count,
            animation_value_scalar_count,
            animation_in_tangent_scalar_count,
            animation_out_tangent_scalar_count,
            profile_resource_count,
            profile_source_count,
            profile_provenance_node_count,
            profile_operation_count,
            profile_evidence_count,
            profile_owner_count,
            profile_attachment_count,
            profile_metadata_entry_count,
            profile_parent_reference_count,
            profile_wide_value_count,
            profile_lineage_text_bytes,
            profile_lineage_digest_bytes,
            profile_metadata_text_bytes,
            profile_metadata_payload_bytes,
            profile_metadata_bool_scalar_count,
            profile_metadata_i64_scalar_count,
            profile_metadata_u64_scalar_count,
            profile_metadata_f64_scalar_count,
            profile_metadata_f32_scalar_count,
            transcript_bytes,
            work_units,
        } = totals;
        Self {
            node_count,
            root_count,
            edge_count,
            camera_count,
            light_count,
            camera_attachment_count,
            light_attachment_count,
            image_count,
            sampler_count,
            texture_count,
            material_count,
            material_binding_count,
            image_payload_bytes,
            media_type_bytes,
            source_key_bytes,
            binding_semantic_scalar_count,
            material_factor_scalar_count,
            logical_mesh_count,
            primitive_count,
            attribute_count,
            mesh_instance_count,
            polygon_boundary_count,
            primitive_index_count,
            sparse_index_count,
            canonical_scalar_count,
            f32_scalar_count,
            u32_scalar_count,
            skin_count,
            logical_morph_count,
            morph_definition_count,
            primitive_deformation_count,
            morph_target_count,
            morph_delta_count,
            deformation_mesh_instance_count,
            skin_joint_count,
            inverse_bind_matrix_count,
            skin_correction_matrix_count,
            skin_name_bytes,
            morph_default_weight_count,
            morph_name_bytes,
            influence_row_count,
            influence_count,
            morph_value_vector_count,
            morph_sparse_index_count,
            instance_morph_weight_count,
            animation_clip_count,
            animation_channel_count,
            animation_time_scalar_count,
            animation_value_scalar_count,
            animation_in_tangent_scalar_count,
            animation_out_tangent_scalar_count,
            profile_resource_count,
            profile_source_count,
            profile_provenance_node_count,
            profile_operation_count,
            profile_evidence_count,
            profile_owner_count,
            profile_attachment_count,
            profile_metadata_entry_count,
            profile_parent_reference_count,
            profile_wide_value_count,
            profile_lineage_text_bytes,
            profile_lineage_digest_bytes,
            profile_metadata_text_bytes,
            profile_metadata_payload_bytes,
            profile_metadata_bool_scalar_count,
            profile_metadata_i64_scalar_count,
            profile_metadata_u64_scalar_count,
            profile_metadata_f64_scalar_count,
            profile_metadata_f32_scalar_count,
            transcript_bytes,
            work_units,
        }
    }
}

#[derive(Serialize)]
pub(super) struct CapabilityBoundedPreviewReport {
    pub(super) schema_id: &'static str,
    pub(super) bytes: usize,
    pub(super) sha256: String,
    pub(super) items_omitted_by_limit: u64,
    pub(super) bytes_truncated: bool,
    pub(super) items_truncated: bool,
}

#[derive(Serialize)]
pub(super) struct GltfCapabilityReport {
    pub(super) entry_point: &'static str,
    pub(super) scenario_id: &'static str,
    pub(super) source_shape: GltfSourceShapeReport,
    pub(super) canonical_shape: GltfCanonicalShapeReport,
    pub(super) source_vector: FixedVectorReport,
    pub(super) mutations: Vec<CapabilityMutationReport>,
    pub(super) source_generation_reservations: u64,
    pub(super) canonical_block_count: usize,
    pub(super) outcome_evidence_block_count: usize,
    pub(super) full_snapshot_digest_hex: String,
    pub(super) bounded_snapshot_digest_hex: String,
    pub(super) snapshot_totals: CanonicalSnapshotTotalsReport,
    pub(super) bounded_preview: CapabilityBoundedPreviewReport,
    pub(super) evidence: EvidenceReport,
    pub(super) construction: PhaseReport,
    pub(super) inspection: PhaseReport,
    pub(super) snapshot: PhaseReport,
    pub(super) unchanged_view: GltfUnchangedViewReport,
    pub(super) changed_scene_edit: GltfChangedSceneEditReport,
    pub(super) release: ReleaseReport,
    pub(super) physical_account: CapabilityPhysicalReport,
    pub(super) factory_after_account_release: PhysicalFactoryReport,
}

#[derive(Serialize)]
pub(super) struct GltfSourceShapeReport {
    pub(super) node_count: usize,
    pub(super) root_count: usize,
    pub(super) image_count: usize,
    pub(super) image_payload_bytes: usize,
    pub(super) media_type_bytes: usize,
    pub(super) sampler_count: usize,
    pub(super) texture_count: usize,
    pub(super) material_count: usize,
    pub(super) material_binding_count: usize,
    pub(super) logical_mesh_count: usize,
    pub(super) primitive_count: usize,
    pub(super) attribute_count: usize,
    pub(super) mesh_instance_count: usize,
    pub(super) skin_count: usize,
    pub(super) skin_joint_count: usize,
    pub(super) morph_definition_count: usize,
    pub(super) morph_target_count: usize,
    pub(super) morph_delta_count: usize,
    pub(super) influence_row_count: usize,
    pub(super) influence_count: usize,
    pub(super) animation_clip_count: usize,
    pub(super) animation_channel_count: usize,
    pub(super) animation_key_count: usize,
}

#[derive(Serialize)]
pub(super) struct GltfCanonicalShapeReport {
    pub(super) node_count: usize,
    pub(super) root_count: usize,
    pub(super) edge_count: usize,
    pub(super) image_count: usize,
    pub(super) sampler_count: usize,
    pub(super) texture_count: usize,
    pub(super) material_count: usize,
    pub(super) material_binding_count: usize,
    pub(super) logical_mesh_count: usize,
    pub(super) primitive_count: usize,
    pub(super) attribute_count: usize,
    pub(super) mesh_instance_count: usize,
    pub(super) skin_count: usize,
    pub(super) skin_joint_count: usize,
    pub(super) logical_morph_count: usize,
    pub(super) morph_definition_count: usize,
    pub(super) primitive_deformation_count: usize,
    pub(super) morph_target_count: usize,
    pub(super) morph_delta_count: usize,
    pub(super) influence_row_count: usize,
    pub(super) influence_count: usize,
    pub(super) animation_clip_count: usize,
    pub(super) animation_channel_count: usize,
    pub(super) animation_key_count: usize,
}

#[derive(Serialize)]
pub(super) struct GltfUnchangedViewReport {
    pub(super) observation: GltfCanonicalShapeReport,
    pub(super) repeated_observation_matches: bool,
    pub(super) generation_preserved: bool,
    pub(super) generation_reservations_before: u64,
    pub(super) generation_reservations_after: u64,
    pub(super) canonical_block_count_before: usize,
    pub(super) canonical_block_count_after: usize,
    pub(super) phase: PhaseReport,
    pub(super) physical_account: ReadOnlyPhysicalReport,
}

#[derive(Serialize)]
pub(super) struct GltfChangedSceneEditReport {
    pub(super) entry_point: &'static str,
    pub(super) scenario_id: &'static str,
    pub(super) output_shape: GltfCanonicalShapeReport,
    pub(super) replacement_authored_bits_preserved: bool,
    pub(super) input_authored_bits_preserved: bool,
    pub(super) input_readable_after_publication: bool,
    pub(super) output_report_readable_after_input_release: bool,
    pub(super) output_readable_after_input_release: bool,
    pub(super) output_readable_after_report_release: bool,
    pub(super) exact_physical_account_preserved: bool,
    pub(super) handoff_physical_account_preserved: bool,
    pub(super) generation_changed: bool,
    pub(super) generation_reservations_before: u64,
    pub(super) generation_reservations_after: u64,
    pub(super) canonical_block_count_before: usize,
    pub(super) canonical_block_count_after: usize,
    pub(super) all_subjects_explicitly_unchanged: bool,
    pub(super) target_rebind_matches: bool,
    pub(super) remap_storage_bytes: u64,
    pub(super) scene_bounds_invalidated: bool,
    pub(super) outcome_evidence_block_count: usize,
    pub(super) evidence: EvidenceReport,
    pub(super) phase: PhaseReport,
    pub(super) input_snapshot_digest_hex: String,
    pub(super) output_snapshot_digest_hex: String,
    pub(super) snapshot: PhaseReport,
    pub(super) input_release: ReleaseReport,
    pub(super) output_report_release: ReleaseReport,
    pub(super) output_scene_release: ReleaseReport,
    pub(super) physical_account: GltfChangedSceneEditPhysicalReport,
}

#[derive(Serialize)]
pub(super) struct GltfChangedSceneEditPhysicalReport {
    pub(super) before: PhysicalAccountReport,
    pub(super) before_handoff: PhysicalAccountReport,
    pub(super) with_output: PhysicalAccountReport,
    pub(super) after_input_release: PhysicalAccountReport,
    pub(super) after_output_report_release: PhysicalAccountReport,
    pub(super) after_output_scene_release: PhysicalAccountReport,
}

#[derive(Serialize)]
pub(super) struct ShareGenerationReport {
    pub(super) construction: PhaseReport,
    pub(super) evidence: EvidenceReport,
    pub(super) canonical_block_count: usize,
}

#[derive(Serialize)]
pub(super) struct HotLeafObservationReport {
    pub(super) phase: PhaseReport,
    pub(super) matrix_bits: [u32; 16],
}

#[derive(Serialize)]
pub(super) struct ValidationFaultReport {
    pub(super) scenario_id: &'static str,
    pub(super) field_class: &'static str,
    pub(super) mutation_schema_id: &'static str,
    pub(super) mutation_transcript_bytes: usize,
    pub(super) mutation_transcript_sha256: String,
    pub(super) image_count: usize,
    pub(super) generation_reservations: u64,
    pub(super) evidence: EvidenceReport,
    pub(super) construction: PhaseReport,
    pub(super) release: ReleaseReport,
    pub(super) physical_account: FaultPhysicalReport,
    pub(super) factory_after_account_release: PhysicalFactoryReport,
}

#[derive(Serialize)]
pub(super) struct MinimalFallbackReport {
    pub(super) scenario_id: &'static str,
    pub(super) fallback_code: u32,
    pub(super) fallback_output_count: usize,
    pub(super) evidence: EvidenceReport,
    pub(super) construction: PhaseReport,
    pub(super) handoff: HandoffReport,
    pub(super) release: ReleaseReport,
    pub(super) physical_account: FaultPhysicalReport,
    pub(super) factory_after_account_release: PhysicalFactoryReport,
}

#[derive(Serialize)]
pub(super) struct ProtectedTerminalActivationReport {
    pub(super) scenario_id: &'static str,
    pub(super) evidence: EvidenceReport,
    pub(super) construction: PhaseReport,
    pub(super) handoff: HandoffReport,
    pub(super) release: ReleaseReport,
    pub(super) physical_account: FaultPhysicalReport,
    pub(super) factory_after_account_release: PhysicalFactoryReport,
}

#[derive(Serialize)]
pub(super) struct PhaseReport {
    scenario_id: &'static str,
    taxonomy: MeasurementTaxonomy,
    allocations: AllocationReport,
}

#[derive(Serialize)]
pub(super) struct ReleaseReport {
    scenario_id: &'static str,
    allocations: AllocationReport,
}

#[derive(Serialize)]
pub(super) struct HandoffReport {
    scenario_id: &'static str,
    allocations: AllocationReport,
}

#[derive(Serialize)]
struct MeasurementTaxonomy {
    semantic_cell_writes: u64,
    reference_writes: u64,
    producer_events: u64,
    payload_copy_bytes: u64,
    payload_backing_allocations: u64,
    block_begins: u64,
    block_seals: u64,
    slot_publications: u64,
    new_block_begins: u64,
    new_block_seals: u64,
    new_slot_publications: u64,
    inherited_block_begins: u64,
    inherited_block_seals: u64,
    inherited_slot_publications: u64,
    element_decodes: u64,
    completed_lookups: u64,
    rejected_lookups: u64,
    lookup_events: u64,
    max_lookup_events: u64,
    member_probes: u64,
    max_member_probes_per_lookup: u64,
}

#[derive(Serialize)]
struct AllocationReport {
    count_total: u64,
    count_current: i64,
    count_max: u64,
    bytes_total: u64,
    bytes_current: i64,
    bytes_max: u64,
}

#[derive(Serialize)]
pub(super) struct EvidenceReport {
    retained_evidence_block_count: usize,
    ledger_header: LedgerHeaderReport,
    dimensions: Vec<LedgerDimensionReport>,
}

#[derive(Serialize)]
struct LedgerDimensionReport {
    role: LedgerDimensionRole,
    semantic_id_hex: String,
    attempted: u64,
    consumed: u64,
    current_live_bytes: u64,
    peak_live_bytes: u64,
    pending_publication: u64,
    committed: u64,
    committed_canonical_output: u64,
    published_outcome_evidence: u64,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
enum LedgerDimensionRole {
    Ordinary,
    FailureTerminal,
}

#[derive(Serialize)]
struct LedgerHeaderReport {
    schema_revision: u32,
    terminal_cause: TerminalCauseReport,
    dimension_count: usize,
    phase_count: usize,
    current_live_bytes: u64,
    peak_live_bytes: u64,
    active_reservations: u64,
    current_evidence_bytes: u64,
    peak_evidence_bytes: u64,
}

#[derive(Serialize)]
struct TerminalCauseReport {
    class: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_class: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<u32>,
}

#[derive(Serialize)]
pub(super) struct SemanticTranscriptReport {
    pub(super) schema_id: &'static str,
    pub(super) observed_bytes: usize,
    pub(super) sha256: String,
    pub(super) bounded_preview: String,
}

#[derive(Serialize)]
pub(super) struct ScalePhysicalReport {
    pub(super) foreign_baseline: PhysicalAccountReport,
    pub(super) target_retained: PhysicalAccountReport,
    pub(super) after_typed_inspection: PhysicalAccountReport,
    pub(super) after_target_release: PhysicalAccountReport,
    pub(super) after_foreign_release: PhysicalAccountReport,
}

#[derive(Serialize)]
pub(super) struct SharePhysicalReport {
    pub(super) baseline: PhysicalAccountReport,
    pub(super) fresh_handoff: PhysicalAccountReport,
    pub(super) fresh_scene_only: PhysicalAccountReport,
    pub(super) derived_handoff: PhysicalAccountReport,
    pub(super) parent_and_derived_scenes: PhysicalAccountReport,
    pub(super) after_parent_release: PhysicalAccountReport,
    pub(super) after_derived_release: PhysicalAccountReport,
}

#[derive(Serialize)]
pub(super) struct FaultPhysicalReport {
    pub(super) baseline: PhysicalAccountReport,
    pub(super) retained: PhysicalAccountReport,
    pub(super) after_handoff: PhysicalAccountReport,
    pub(super) after_release: PhysicalAccountReport,
}

#[derive(Serialize)]
pub(super) struct CapabilityPhysicalReport {
    pub(super) baseline: PhysicalAccountReport,
    pub(super) retained: PhysicalAccountReport,
    pub(super) after_inspection: PhysicalAccountReport,
    pub(super) after_snapshot: PhysicalAccountReport,
    pub(super) after_release: PhysicalAccountReport,
}

#[derive(Serialize)]
pub(super) struct ReadOnlyPhysicalReport {
    pub(super) before: PhysicalAccountReport,
    pub(super) after: PhysicalAccountReport,
}

#[derive(Serialize)]
pub(super) struct PhysicalAccountReport {
    live_retained_bytes: u64,
    live_retained_allocations: u64,
    peak_retained_bytes: u64,
    peak_retained_allocations: u64,
    total_admitted_retained_bytes: u64,
    total_admitted_retained_allocations: u64,
    poisoned: bool,
    poison_event_count: u128,
    poison_event_count_exact: bool,
    poison_event_domain_valid: bool,
}

#[derive(Serialize)]
pub(super) struct PhysicalFactoryReport {
    live_account_control_bytes: u64,
    live_accounts: u64,
    peak_account_control_bytes: u64,
    peak_live_accounts: u64,
    total_admitted_account_control_bytes: u64,
    total_admitted_accounts: u64,
}

pub(super) fn phase_report(
    scenario_id: &'static str,
    measurement: MeasurementReport,
    allocations: AllocationInfo,
) -> PhaseReport {
    PhaseReport {
        scenario_id,
        taxonomy: measurement_taxonomy(measurement),
        allocations: allocations.into(),
    }
}

pub(super) fn release_report(
    scenario_id: &'static str,
    allocations: AllocationInfo,
) -> ReleaseReport {
    ReleaseReport {
        scenario_id,
        allocations: allocations.into(),
    }
}

pub(super) fn handoff_report(
    scenario_id: &'static str,
    allocations: AllocationInfo,
) -> HandoffReport {
    HandoffReport {
        scenario_id,
        allocations: allocations.into(),
    }
}

pub(super) fn evidence_report(
    retained_evidence_block_count: usize,
    evidence: &StoreLedgerEvidence,
) -> EvidenceReport {
    let header = evidence.header();
    assert_eq!(
        header.dimension_count(),
        2,
        "U4c production V2 ledger evidence must contain exactly the ordinary and failure-terminal dimensions",
    );
    let mut saw_ordinary = false;
    let mut saw_failure_terminal = false;
    let dimensions = (0..header.dimension_count())
        .map(|index| {
            let dimension = evidence
                .dimension_at(index)
                .expect("ledger evidence header must enumerate every dimension");
            let semantic_id = dimension.dimension();
            let role = if semantic_id == STORE_DIMENSION {
                assert!(
                    !saw_ordinary,
                    "U4c production V2 ledger evidence must not duplicate the ordinary dimension",
                );
                saw_ordinary = true;
                LedgerDimensionRole::Ordinary
            } else if semantic_id == FAILURE_TERMINAL_WORK_DIMENSION {
                assert!(
                    !saw_failure_terminal,
                    "U4c production V2 ledger evidence must not duplicate the failure-terminal dimension",
                );
                saw_failure_terminal = true;
                LedgerDimensionRole::FailureTerminal
            } else {
                panic!(
                    "U4c production V2 ledger evidence contains unknown dimension {}",
                    hex_encode(&semantic_id.to_canonical_bytes()),
                );
            };
            LedgerDimensionReport {
                role,
                semantic_id_hex: hex_encode(&semantic_id.to_canonical_bytes()),
                attempted: dimension.attempted(),
                consumed: dimension.consumed(),
                current_live_bytes: dimension.current_live_bytes(),
                peak_live_bytes: dimension.peak_live_bytes(),
                pending_publication: dimension.pending_publication(),
                committed: dimension.committed(),
                committed_canonical_output: dimension.committed_canonical_output(),
                published_outcome_evidence: dimension.published_outcome_evidence(),
            }
        })
        .collect();
    assert!(
        saw_ordinary && saw_failure_terminal,
        "U4c production V2 ledger evidence must contain both the ordinary and failure-terminal dimensions",
    );
    EvidenceReport {
        retained_evidence_block_count,
        ledger_header: LedgerHeaderReport {
            schema_revision: header.schema_revision(),
            terminal_cause: terminal_cause_report(header.terminal_cause()),
            dimension_count: header.dimension_count(),
            phase_count: header.phase_count(),
            current_live_bytes: header.current_live_bytes(),
            peak_live_bytes: header.peak_live_bytes(),
            active_reservations: header.active_reservations(),
            current_evidence_bytes: header.current_evidence_bytes(),
            peak_evidence_bytes: header.peak_evidence_bytes(),
        },
        dimensions,
    }
}

fn terminal_cause_report(cause: StoreTerminalCause) -> TerminalCauseReport {
    match cause {
        StoreTerminalCause::Success => TerminalCauseReport {
            class: "success",
            failure_class: None,
            code: None,
        },
        StoreTerminalCause::Failure(failure) => TerminalCauseReport {
            class: "failure",
            failure_class: Some(match failure.class() {
                StorePreparationFailureClass::Requested => "requested",
                StorePreparationFailureClass::Control => "control",
                StorePreparationFailureClass::ProductGraph => "product_graph",
                StorePreparationFailureClass::Ledger => "ledger",
                _ => "unknown",
            }),
            code: Some(failure.code()),
        },
        _ => TerminalCauseReport {
            class: "unknown",
            failure_class: None,
            code: None,
        },
    }
}

fn measurement_taxonomy(report: MeasurementReport) -> MeasurementTaxonomy {
    MeasurementTaxonomy {
        semantic_cell_writes: report.semantic_cell_writes(),
        reference_writes: report.reference_writes(),
        producer_events: report.producer_events(),
        payload_copy_bytes: report.payload_copy_bytes(),
        payload_backing_allocations: report.payload_backing_allocations(),
        block_begins: report.block_begins(),
        block_seals: report.block_seals(),
        slot_publications: report.slot_publications(),
        new_block_begins: report.new_block_begins(),
        new_block_seals: report.new_block_seals(),
        new_slot_publications: report.new_slot_publications(),
        inherited_block_begins: report.inherited_block_begins(),
        inherited_block_seals: report.inherited_block_seals(),
        inherited_slot_publications: report.inherited_slot_publications(),
        element_decodes: report.element_decodes(),
        completed_lookups: report.completed_lookups(),
        rejected_lookups: report.rejected_lookups(),
        lookup_events: report.lookup_events(),
        max_lookup_events: report.max_lookup_events(),
        member_probes: report.member_probes(),
        max_member_probes_per_lookup: report.max_member_probes_per_lookup(),
    }
}

impl From<AllocationInfo> for AllocationReport {
    fn from(info: AllocationInfo) -> Self {
        Self {
            count_total: info.count_total,
            count_current: info.count_current,
            count_max: info.count_max,
            bytes_total: info.bytes_total,
            bytes_current: info.bytes_current,
            bytes_max: info.bytes_max,
        }
    }
}

impl From<PhysicalStorageAccountSnapshot> for PhysicalAccountReport {
    fn from(snapshot: PhysicalStorageAccountSnapshot) -> Self {
        Self {
            live_retained_bytes: snapshot.live_retained_bytes(),
            live_retained_allocations: snapshot.live_retained_allocations(),
            peak_retained_bytes: snapshot.peak_retained_bytes(),
            peak_retained_allocations: snapshot.peak_retained_allocations(),
            total_admitted_retained_bytes: snapshot.total_admitted_retained_bytes(),
            total_admitted_retained_allocations: snapshot.total_admitted_retained_allocations(),
            poisoned: snapshot.poisoned(),
            poison_event_count: snapshot.poison_event_count(),
            poison_event_count_exact: snapshot.poison_event_count_exact(),
            poison_event_domain_valid: snapshot.poison_event_domain_valid(),
        }
    }
}

impl From<PhysicalStorageAccountFactorySnapshot> for PhysicalFactoryReport {
    fn from(snapshot: PhysicalStorageAccountFactorySnapshot) -> Self {
        Self {
            live_account_control_bytes: snapshot.live_account_control_bytes(),
            live_accounts: snapshot.live_accounts(),
            peak_account_control_bytes: snapshot.peak_account_control_bytes(),
            peak_live_accounts: snapshot.peak_live_accounts(),
            total_admitted_account_control_bytes: snapshot.total_admitted_account_control_bytes(),
            total_admitted_accounts: snapshot.total_admitted_accounts(),
        }
    }
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_encode(&digest)
}

pub(super) fn hex_encode(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(hex, "{byte:02x}").expect("writing digest hex cannot fail");
    }
    hex
}

pub(super) fn render(report: &ProductionReport) -> String {
    let rendered = serde_json::to_string(report).expect("production report must serialize");
    assert!(rendered.is_ascii());
    assert!(!rendered.contains('\n'));
    assert!(!rendered.contains('\r'));
    let _: serde_json::Value =
        serde_json::from_str(&rendered).expect("production report must be valid JSON");
    rendered
}

#[test]
fn production_report_contract_is_v2() {
    assert_eq!(REPORT_SCHEMA_ID, "baozi.u4c.production-report");
    assert_eq!(REPORT_SCHEMA_VERSION, 2);
    assert_eq!(REPORT_REVISION, 1);
    assert_eq!(SCENARIO_SET_ID, "canonical-store-production-v2");
}
