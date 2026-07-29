use std::fmt::Write as _;

use allocation_counter::AllocationInfo;
use baozi_operation::__u4c::MeasurementReport;
use baozi_operation::{
    PhysicalStorageAccountFactorySnapshot, PhysicalStorageAccountSnapshot, StoreLedgerEvidence,
    StorePreparationFailureClass, StoreTerminalCause,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub(super) const REPORT_PREFIX: &str = "BAOZI_U4C_PRODUCTION_V1=";
const REPORT_SCHEMA_ID: &str = "baozi.u4c.production-report";
const REPORT_SCHEMA_VERSION: u32 = 1;
const REPORT_REVISION: u32 = 5;
const SCENARIO_SET_ID: &str = "canonical-store-production-v1";

#[derive(Serialize)]
pub(super) struct ProductionReport {
    schema_id: &'static str,
    schema_version: u32,
    report_revision: u32,
    scenario_set_id: &'static str,
    scale_scenarios: Vec<ScaleScenarioReport>,
    inherited_hot_leaf_scenario: InheritedHotLeafReport,
    obj_capability: ObjCapabilityReport,
    validation_faults: Vec<ValidationFaultReport>,
}

impl ProductionReport {
    pub(super) fn new(
        scale_scenarios: Vec<ScaleScenarioReport>,
        inherited_hot_leaf_scenario: InheritedHotLeafReport,
        obj_capability: ObjCapabilityReport,
        validation_faults: Vec<ValidationFaultReport>,
    ) -> Self {
        Self {
            schema_id: REPORT_SCHEMA_ID,
            schema_version: REPORT_SCHEMA_VERSION,
            report_revision: REPORT_REVISION,
            scenario_set_id: SCENARIO_SET_ID,
            scale_scenarios,
            inherited_hot_leaf_scenario,
            obj_capability,
            validation_faults,
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
    pub(super) mutations: Vec<ObjMutationReport>,
    pub(super) generation_reservations: u64,
    pub(super) canonical_block_count: usize,
    pub(super) outcome_evidence_block_count: usize,
    pub(super) full_snapshot_digest_hex: String,
    pub(super) bounded_snapshot_digest_hex: String,
    pub(super) snapshot_totals: ObjSnapshotTotalsReport,
    pub(super) bounded_preview: ObjBoundedPreviewReport,
    pub(super) evidence: EvidenceReport,
    pub(super) construction: PhaseReport,
    pub(super) inspection: PhaseReport,
    pub(super) snapshot: PhaseReport,
    pub(super) release: ReleaseReport,
    pub(super) physical_account: ObjPhysicalReport,
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
pub(super) struct ObjMutationReport {
    pub(super) field_class: &'static str,
    pub(super) source_vector: FixedVectorReport,
    pub(super) snapshot_digest_hex: String,
}

#[derive(Serialize)]
pub(super) struct ObjSnapshotTotalsReport {
    pub(super) node_count: usize,
    pub(super) root_count: usize,
    pub(super) edge_count: usize,
    pub(super) material_count: usize,
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
    pub(super) transcript_bytes: u64,
    pub(super) work_units: u64,
}

#[derive(Serialize)]
pub(super) struct ObjBoundedPreviewReport {
    pub(super) schema_id: &'static str,
    pub(super) bytes: usize,
    pub(super) sha256: String,
    pub(super) items_omitted_by_limit: u64,
    pub(super) bytes_truncated: bool,
    pub(super) items_truncated: bool,
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
    outcome_evidence_block_count: usize,
    ledger_header: LedgerHeaderReport,
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
pub(super) struct ObjPhysicalReport {
    pub(super) baseline: PhysicalAccountReport,
    pub(super) retained: PhysicalAccountReport,
    pub(super) after_inspection: PhysicalAccountReport,
    pub(super) after_snapshot: PhysicalAccountReport,
    pub(super) after_release: PhysicalAccountReport,
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

pub(super) fn evidence_report(
    outcome_evidence_block_count: usize,
    evidence: &StoreLedgerEvidence,
) -> EvidenceReport {
    let header = evidence.header();
    EvidenceReport {
        outcome_evidence_block_count,
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
