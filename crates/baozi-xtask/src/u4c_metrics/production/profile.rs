use std::collections::BTreeSet;

use serde::Deserialize;

use super::capability::{
    BoundedPreview, ConstructionBudgetSpecification, SnapshotBudgetSpecification,
};
#[cfg(feature = "repository-input-candidate")]
use super::capability::{
    PhysicalLifecycleReport, validate_allocation_lifecycle, validate_nonproducing_phase,
};
use super::{CanonicalSnapshotTotals, is_lowercase_sha256};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProfileClosureSpecification {
    pub(super) entry_point: String,
    pub(super) scenario_id: String,
    pub(super) construction_scenario_id: String,
    pub(super) inspection_scenario_id: String,
    pub(super) snapshot_scenario_id: String,
    pub(super) release_scenario_id: String,
    pub(super) canonical_shape: ProfileClosureCanonicalShape,
    pub(super) mutations: Vec<ProfileClosureMutation>,
    pub(super) negative_cases: Vec<ProfileClosureNegativeCase>,
    pub(super) generation_reservations: u64,
    pub(super) canonical_block_count: u64,
    pub(super) outcome_evidence_block_count: u64,
    pub(super) full_snapshot_digest_hex: String,
    pub(super) bounded_snapshot_digest_hex: String,
    pub(super) snapshot_totals: CanonicalSnapshotTotals,
    pub(super) bounded_preview: BoundedPreview,
    pub(super) construction_budget: ConstructionBudgetSpecification,
    pub(super) snapshot_budget: SnapshotBudgetSpecification,
    pub(super) construction_semantic_cells: u64,
    pub(super) construction_references: u64,
    pub(super) construction_blocks: u64,
    pub(super) construction_producer_events: u64,
    pub(super) inspection_completed_lookups: u64,
    pub(super) inspection_lookup_events: u64,
    pub(super) inspection_member_probes: u64,
    pub(super) retained_bytes: u64,
    pub(super) retained_allocations: u64,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct ProfileClosureCanonicalShape {
    pub(super) node_count: u64,
    pub(super) root_count: u64,
    pub(super) camera_count: u64,
    pub(super) light_count: u64,
    pub(super) camera_attachment_count: u64,
    pub(super) light_attachment_count: u64,
    pub(super) unit_scale_to_meters: f64,
    pub(super) profile_resource_count: u64,
    pub(super) profile_source_count: u64,
    pub(super) profile_provenance_count: u64,
    pub(super) profile_owner_count: u64,
    pub(super) profile_attachment_count: u64,
    pub(super) profile_metadata_entry_count: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProfileClosureMutation {
    pub(super) field_class: String,
    pub(super) snapshot_digest_hex: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProfileClosureNegativeCase {
    pub(super) scenario_id: String,
    pub(super) preflight_disposition: String,
    pub(super) activation_eligibility_reached: bool,
    pub(super) failure_class: String,
    pub(super) failure_kind: String,
    pub(super) failure_code: u32,
    pub(super) peak_retained_bytes_delta: u64,
    pub(super) peak_retained_allocations_delta: u64,
    pub(super) total_admitted_retained_bytes_delta: u64,
    pub(super) total_admitted_retained_allocations_delta: u64,
    pub(super) live_retained_bytes_delta: u64,
    pub(super) live_retained_allocations_delta: u64,
    pub(super) final_live_retained_bytes: u64,
    pub(super) final_live_retained_allocations: u64,
}

pub(super) fn validate_specification(
    specification: &ProfileClosureSpecification,
) -> Result<(), String> {
    let shape = &specification.canonical_shape;
    let totals = &specification.snapshot_totals;
    let preview = &specification.bounded_preview;
    let phase_scenarios = [
        specification.scenario_id.as_str(),
        specification.construction_scenario_id.as_str(),
        specification.inspection_scenario_id.as_str(),
        specification.snapshot_scenario_id.as_str(),
        specification.release_scenario_id.as_str(),
    ];
    let published_block_count = specification
        .canonical_block_count
        .checked_add(specification.outcome_evidence_block_count);
    if !is_bounded_identity(&specification.entry_point)
        || phase_scenarios.iter().any(|id| !is_bounded_identity(id))
        || phase_scenarios
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len()
            != phase_scenarios.len()
        || specification.generation_reservations == 0
        || specification.canonical_block_count == 0
        || specification.outcome_evidence_block_count == 0
        || published_block_count != Some(specification.construction_blocks)
        || !is_lowercase_sha256(&specification.full_snapshot_digest_hex)
        || specification.bounded_snapshot_digest_hex != specification.full_snapshot_digest_hex
        || !profile_shape_is_well_formed(shape)
        || !profile_snapshot_totals_match(totals, shape)
        || !preview.has_common_invariants()
        || !specification.construction_budget.is_well_formed()
        || !specification.snapshot_budget.is_well_formed()
        || specification.construction_semantic_cells == 0
        || specification.construction_references == 0
        || specification.construction_blocks == 0
        || specification.construction_producer_events == 0
        || specification.inspection_completed_lookups == 0
        || specification.inspection_lookup_events < specification.inspection_completed_lookups
        || specification.inspection_member_probes > specification.inspection_lookup_events
        || specification.retained_bytes == 0
        || specification.retained_allocations == 0
    {
        return Err(
            "U4c production profile-closure specification is not the frozen canonical vector"
                .to_owned(),
        );
    }
    if specification.mutations.is_empty() {
        return Err("U4c production profile-closure must freeze a mutation vector".to_owned());
    }
    let mut mutation_fields = BTreeSet::new();
    for mutation in &specification.mutations {
        if !is_bounded_identity(&mutation.field_class)
            || !is_lowercase_sha256(&mutation.snapshot_digest_hex)
            || mutation.snapshot_digest_hex == specification.full_snapshot_digest_hex
            || !mutation_fields.insert(mutation.field_class.as_str())
        {
            return Err(
                "U4c production profile-closure mutation evidence is invalid or duplicated"
                    .to_owned(),
            );
        }
    }
    validate_negative_cases(&specification.negative_cases)?;
    Ok(())
}

fn is_bounded_identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

fn profile_shape_is_well_formed(shape: &ProfileClosureCanonicalShape) -> bool {
    shape.node_count > 0
        && shape.root_count > 0
        && shape.root_count <= shape.node_count
        && shape.camera_count > 0
        && shape.light_count > 0
        && shape.camera_attachment_count > 0
        && shape.light_attachment_count > 0
        && shape.unit_scale_to_meters.is_finite()
        && shape.unit_scale_to_meters > 0.0
        && shape.profile_resource_count > 0
        && shape.profile_source_count > 0
        && shape.profile_provenance_count > 0
        && shape.profile_owner_count > 0
        && shape.profile_attachment_count == shape.profile_owner_count
        && shape.profile_metadata_entry_count > 0
}

fn profile_snapshot_totals_match(
    totals: &CanonicalSnapshotTotals,
    shape: &ProfileClosureCanonicalShape,
) -> bool {
    totals.node_count == shape.node_count
        && totals.root_count == shape.root_count
        && totals.edge_count
            == shape
                .node_count
                .checked_sub(shape.root_count)
                .unwrap_or(u64::MAX)
        && totals.camera_count == shape.camera_count
        && totals.light_count == shape.light_count
        && totals.camera_attachment_count == shape.camera_attachment_count
        && totals.light_attachment_count == shape.light_attachment_count
        && totals.image_count == 0
        && totals.sampler_count == 0
        && totals.texture_count == 0
        && totals.material_count == 0
        && totals.material_binding_count == 0
        && totals.image_payload_bytes == 0
        && totals.media_type_bytes == 0
        && totals.source_key_bytes == 0
        && totals.binding_semantic_scalar_count == 0
        && totals.material_factor_scalar_count == 0
        && totals.logical_mesh_count == 0
        && totals.primitive_count == 0
        && totals.attribute_count == 0
        && totals.mesh_instance_count == 0
        && totals.polygon_boundary_count == 0
        && totals.primitive_index_count == 0
        && totals.sparse_index_count == 0
        && totals.canonical_scalar_count == 0
        && totals.f32_scalar_count == 0
        && totals.u32_scalar_count == 0
        && totals.skin_count == 0
        && totals.logical_morph_count == 0
        && totals.morph_definition_count == 0
        && totals.primitive_deformation_count == 0
        && totals.morph_target_count == 0
        && totals.morph_delta_count == 0
        && totals.deformation_mesh_instance_count == 0
        && totals.skin_joint_count == 0
        && totals.inverse_bind_matrix_count == 0
        && totals.skin_correction_matrix_count == 0
        && totals.skin_name_bytes == 0
        && totals.morph_default_weight_count == 0
        && totals.morph_name_bytes == 0
        && totals.influence_row_count == 0
        && totals.influence_count == 0
        && totals.morph_value_vector_count == 0
        && totals.morph_sparse_index_count == 0
        && totals.instance_morph_weight_count == 0
        && totals.animation_clip_count == 0
        && totals.animation_channel_count == 0
        && totals.animation_time_scalar_count == 0
        && totals.animation_value_scalar_count == 0
        && totals.animation_in_tangent_scalar_count == 0
        && totals.animation_out_tangent_scalar_count == 0
        && totals.profile_resource_count == shape.profile_resource_count
        && totals.profile_source_count == shape.profile_source_count
        && totals.profile_provenance_node_count == shape.profile_provenance_count
        && totals.profile_operation_count == 0
        && totals.profile_evidence_count == 0
        && totals.profile_owner_count == shape.profile_owner_count
        && totals.profile_attachment_count == shape.profile_attachment_count
        && totals.profile_metadata_entry_count == shape.profile_metadata_entry_count
        && totals.profile_parent_reference_count == 0
        && totals.profile_wide_value_count > 0
        && totals.profile_lineage_text_bytes > 0
        && totals.profile_lineage_digest_bytes == 0
        && totals.profile_metadata_text_bytes > 0
        && totals.profile_metadata_payload_bytes > 0
        && totals
            .profile_metadata_bool_scalar_count
            .checked_add(totals.profile_metadata_i64_scalar_count)
            .and_then(|value| value.checked_add(totals.profile_metadata_u64_scalar_count))
            .and_then(|value| value.checked_add(totals.profile_metadata_f64_scalar_count))
            .and_then(|value| value.checked_add(totals.profile_metadata_f32_scalar_count))
            .is_some_and(|value| value > 0)
        && totals.transcript_bytes > 0
        && totals.work_units > 0
}

fn validate_negative_cases(values: &[ProfileClosureNegativeCase]) -> Result<(), String> {
    if values.is_empty() {
        return Err("U4c production profile-closure must freeze a negative case".to_owned());
    }
    let mut scenario_ids = BTreeSet::new();
    if values.iter().any(|case| {
        !is_bounded_identity(&case.scenario_id)
            || !scenario_ids.insert(case.scenario_id.as_str())
            || !is_bounded_identity(&case.preflight_disposition)
            || case.preflight_disposition != "rejected"
            || case.activation_eligibility_reached
            || !is_bounded_identity(&case.failure_class)
            || !is_bounded_identity(&case.failure_kind)
            || case.failure_code == 0
            || case.peak_retained_bytes_delta != 0
            || case.peak_retained_allocations_delta != 0
            || case.total_admitted_retained_bytes_delta != 0
            || case.total_admitted_retained_allocations_delta != 0
            || case.live_retained_bytes_delta != 0
            || case.live_retained_allocations_delta != 0
            || case.final_live_retained_bytes != 0
            || case.final_live_retained_allocations != 0
    }) {
        return Err(
            "U4c production profile-closure negative cases must freeze rejected preflight, unreachable activation eligibility, zero preflight storage deltas, and zero final live storage"
                .to_owned(),
        );
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProfileClosureReport {
    pub(super) entry_point: String,
    pub(super) scenario_id: String,
    pub(super) canonical_shape: ProfileClosureCanonicalShapeReport,
    pub(super) mutations: Vec<ProfileClosureMutationReport>,
    pub(super) negative_cases: Vec<ProfileClosureNegativeCaseReport>,
    pub(super) generation_reservations: u64,
    pub(super) canonical_block_count: u64,
    pub(super) outcome_evidence_block_count: u64,
    pub(super) full_snapshot_digest_hex: String,
    pub(super) bounded_snapshot_digest_hex: String,
    pub(super) snapshot_totals: CanonicalSnapshotTotals,
    pub(super) bounded_preview: BoundedPreview,
    pub(super) evidence: super::EvidenceReport,
    pub(super) construction: super::PhaseReport,
    pub(super) inspection: super::PhaseReport,
    pub(super) snapshot: super::PhaseReport,
    pub(super) release: super::ReleaseReport,
    pub(super) physical_account: PhysicalLifecycleReport,
    pub(super) factory_after_account_release: super::PhysicalFactoryReport,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct ProfileClosureCanonicalShapeReport {
    pub(super) node_count: u64,
    pub(super) root_count: u64,
    pub(super) camera_count: u64,
    pub(super) light_count: u64,
    pub(super) camera_attachment_count: u64,
    pub(super) light_attachment_count: u64,
    pub(super) unit_scale_to_meters: f64,
    pub(super) profile_resource_count: u64,
    pub(super) profile_source_count: u64,
    pub(super) profile_provenance_count: u64,
    pub(super) profile_owner_count: u64,
    pub(super) profile_attachment_count: u64,
    pub(super) profile_metadata_entry_count: u64,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProfileClosureMutationReport {
    pub(super) field_class: String,
    pub(super) snapshot_digest_hex: String,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProfileClosureNegativeCaseReport {
    pub(super) scenario_id: String,
    pub(super) preflight_disposition: String,
    pub(super) activation_eligibility_reached: bool,
    pub(super) failure_class: String,
    pub(super) failure_kind: String,
    pub(super) failure_code: u32,
    pub(super) peak_retained_bytes_delta: u64,
    pub(super) peak_retained_allocations_delta: u64,
    pub(super) total_admitted_retained_bytes_delta: u64,
    pub(super) total_admitted_retained_allocations_delta: u64,
    pub(super) live_retained_bytes_delta: u64,
    pub(super) live_retained_allocations_delta: u64,
    pub(super) final_live_retained_bytes: u64,
    pub(super) final_live_retained_allocations: u64,
}

#[cfg(feature = "repository-input-candidate")]
fn canonical_shape_matches(
    report: &ProfileClosureCanonicalShapeReport,
    expected: &ProfileClosureCanonicalShape,
) -> bool {
    report.node_count == expected.node_count
        && report.root_count == expected.root_count
        && report.camera_count == expected.camera_count
        && report.light_count == expected.light_count
        && report.camera_attachment_count == expected.camera_attachment_count
        && report.light_attachment_count == expected.light_attachment_count
        && report.unit_scale_to_meters.to_bits() == expected.unit_scale_to_meters.to_bits()
        && report.profile_resource_count == expected.profile_resource_count
        && report.profile_source_count == expected.profile_source_count
        && report.profile_provenance_count == expected.profile_provenance_count
        && report.profile_owner_count == expected.profile_owner_count
        && report.profile_attachment_count == expected.profile_attachment_count
        && report.profile_metadata_entry_count == expected.profile_metadata_entry_count
}

#[cfg(feature = "repository-input-candidate")]
fn mutation_evidence_matches(
    report: &[ProfileClosureMutationReport],
    expected: &[ProfileClosureMutation],
) -> bool {
    report.len() == expected.len()
        && report.iter().zip(expected).all(|(actual, expected)| {
            actual.field_class == expected.field_class
                && actual.snapshot_digest_hex == expected.snapshot_digest_hex
        })
}

#[cfg(feature = "repository-input-candidate")]
fn negative_case_evidence_matches(
    report: &[ProfileClosureNegativeCaseReport],
    expected: &[ProfileClosureNegativeCase],
) -> bool {
    report.len() == expected.len()
        && report.iter().zip(expected).all(|(actual, expected)| {
            actual.scenario_id == expected.scenario_id
                && actual.preflight_disposition == expected.preflight_disposition
                && actual.activation_eligibility_reached == expected.activation_eligibility_reached
                && actual.failure_class == expected.failure_class
                && actual.failure_kind == expected.failure_kind
                && actual.failure_code == expected.failure_code
                && actual.peak_retained_bytes_delta == expected.peak_retained_bytes_delta
                && actual.peak_retained_allocations_delta
                    == expected.peak_retained_allocations_delta
                && actual.total_admitted_retained_bytes_delta
                    == expected.total_admitted_retained_bytes_delta
                && actual.total_admitted_retained_allocations_delta
                    == expected.total_admitted_retained_allocations_delta
                && actual.live_retained_bytes_delta == expected.live_retained_bytes_delta
                && actual.live_retained_allocations_delta
                    == expected.live_retained_allocations_delta
                && actual.final_live_retained_bytes == expected.final_live_retained_bytes
                && actual.final_live_retained_allocations
                    == expected.final_live_retained_allocations
        })
}

#[cfg(feature = "repository-input-candidate")]
fn retained_physical_charge_matches(
    report: &PhysicalLifecycleReport,
    expected: &ProfileClosureSpecification,
) -> bool {
    report.retained.live_retained_bytes == expected.retained_bytes
        && report.retained.live_retained_allocations == expected.retained_allocations
        && report.after_inspection == report.retained
        && report.after_snapshot == report.retained
}

#[cfg(feature = "repository-input-candidate")]
fn outcome_evidence_count_matches(
    retained_evidence_block_count: u64,
    outcome_evidence_block_count: u64,
) -> bool {
    retained_evidence_block_count == outcome_evidence_block_count
}

#[cfg(feature = "repository-input-candidate")]
pub(super) fn validate_report(
    specification: &super::ProductionSpecification,
    report: &ProfileClosureReport,
) -> Result<(), String> {
    let expected = &specification.profile_closure;
    if report.entry_point != expected.entry_point
        || report.scenario_id != expected.scenario_id
        || report.generation_reservations != expected.generation_reservations
        || report.canonical_block_count != expected.canonical_block_count
        || report.outcome_evidence_block_count != expected.outcome_evidence_block_count
        || !outcome_evidence_count_matches(
            report.evidence.retained_evidence_block_count,
            report.outcome_evidence_block_count,
        )
        || report.full_snapshot_digest_hex != expected.full_snapshot_digest_hex
        || report.bounded_snapshot_digest_hex != expected.bounded_snapshot_digest_hex
        || !canonical_shape_matches(&report.canonical_shape, &expected.canonical_shape)
        || report.snapshot_totals != expected.snapshot_totals
        || report.bounded_preview != expected.bounded_preview
    {
        return Err(
            "U4c production profile-closure report drifted from the frozen specification"
                .to_owned(),
        );
    }
    if !mutation_evidence_matches(&report.mutations, &expected.mutations) {
        return Err("U4c production profile-closure mutation evidence drifted".to_owned());
    }
    if !negative_case_evidence_matches(&report.negative_cases, &expected.negative_cases) {
        return Err("U4c production profile-closure negative evidence drifted".to_owned());
    }
    super::validate_success_evidence(
        &specification.ledger_contract,
        &report.evidence,
        expected.canonical_block_count,
        "U4c production profile-closure",
    )?;
    validate_construction_phase(report, expected)?;
    validate_inspection_phase(&report.inspection, expected, &specification.measurement)?;
    validate_snapshot_phase(&report.snapshot, expected)?;
    if report.release.scenario_id != expected.release_scenario_id {
        return Err("U4c production profile-closure release scenario drifted".to_owned());
    }
    super::validate_release(&report.release, "U4c production profile-closure release")?;
    validate_allocation_lifecycle(
        [
            &report.construction.allocations,
            &report.inspection.allocations,
            &report.snapshot.allocations,
            &report.release.allocations,
        ],
        "U4c production profile-closure allocation lifecycle",
    )?;
    super::validate_released_account(
        &report.physical_account.baseline,
        "U4c profile-closure baseline",
    )?;
    super::validate_healthy_account(
        &report.physical_account.retained,
        "U4c profile-closure retained",
    )?;
    if !retained_physical_charge_matches(&report.physical_account, expected) {
        return Err("U4c production profile-closure retained physical charge drifted".to_owned());
    }
    super::validate_released_account(
        &report.physical_account.after_release,
        "U4c profile-closure final release",
    )?;
    super::validate_released_factory(
        &report.factory_after_account_release,
        "U4c profile-closure factory release",
    )
}

#[cfg(feature = "repository-input-candidate")]
fn construction_phase_matches(
    report: &super::PhaseReport,
    expected: &ProfileClosureSpecification,
) -> bool {
    let taxonomy = &report.taxonomy;
    let budget = &expected.construction_budget;
    report.scenario_id == expected.construction_scenario_id
        && taxonomy.semantic_cell_writes == expected.construction_semantic_cells
        && taxonomy.reference_writes == expected.construction_references
        && taxonomy.block_begins == expected.construction_blocks
        && taxonomy.block_seals == expected.construction_blocks
        && taxonomy.slot_publications == expected.construction_blocks
        && taxonomy.new_block_begins == expected.construction_blocks
        && taxonomy.new_block_seals == expected.construction_blocks
        && taxonomy.new_slot_publications == expected.construction_blocks
        && taxonomy.inherited_block_begins == 0
        && taxonomy.inherited_block_seals == 0
        && taxonomy.inherited_slot_publications == 0
        && taxonomy.producer_events == expected.construction_producer_events
        && taxonomy.payload_copy_bytes <= budget.max_payload_copy_bytes
        && taxonomy.payload_backing_allocations == expected.construction_blocks
        && taxonomy.element_decodes == 0
        && taxonomy.completed_lookups == 0
        && taxonomy.rejected_lookups == 0
        && taxonomy.lookup_events == 0
        && taxonomy.max_lookup_events == 0
        && taxonomy.member_probes == 0
        && taxonomy.max_member_probes_per_lookup == 0
        && budget.allocations.contains(&report.allocations)
}

#[cfg(feature = "repository-input-candidate")]
fn validate_construction_phase(
    report: &ProfileClosureReport,
    expected: &ProfileClosureSpecification,
) -> Result<(), String> {
    if !construction_phase_matches(&report.construction, expected) {
        return Err("U4c production profile-closure construction transcript drifted".to_owned());
    }
    super::validate_producer_bound(&report.construction, "U4c profile-closure construction")
}

#[cfg(feature = "repository-input-candidate")]
fn validate_inspection_phase(
    report: &super::PhaseReport,
    expected: &ProfileClosureSpecification,
    measurement: &super::MeasurementSpecification,
) -> Result<(), String> {
    validate_nonproducing_phase(report, "U4c profile-closure inspection")?;
    let taxonomy = &report.taxonomy;
    if report.scenario_id != expected.inspection_scenario_id
        || taxonomy.completed_lookups != expected.inspection_completed_lookups
        || taxonomy.rejected_lookups != 0
        || taxonomy.lookup_events != expected.inspection_lookup_events
        || taxonomy.member_probes != expected.inspection_member_probes
        || taxonomy.max_lookup_events != measurement.max_lookup_events
        || taxonomy.max_member_probes_per_lookup != measurement.max_member_probes
    {
        return Err("U4c production profile-closure inspection transcript drifted".to_owned());
    }
    super::validate_hot_view_phase(report, measurement, "U4c profile-closure inspection")
}

#[cfg(feature = "repository-input-candidate")]
fn validate_snapshot_phase(
    report: &super::PhaseReport,
    expected: &ProfileClosureSpecification,
) -> Result<(), String> {
    validate_nonproducing_phase(report, "U4c profile-closure snapshot")?;
    let taxonomy = &report.taxonomy;
    let budget = &expected.snapshot_budget;
    if report.scenario_id != expected.snapshot_scenario_id
        || taxonomy.completed_lookups != budget.completed_lookups
        || taxonomy.rejected_lookups != 0
        || taxonomy.lookup_events != budget.lookup_events
        || taxonomy.member_probes != budget.member_probes
        || taxonomy.max_lookup_events != budget.max_lookup_events
        || taxonomy.max_member_probes_per_lookup != budget.max_member_probes_per_lookup
        || !budget.allocations.contains(&report.allocations)
    {
        return Err("U4c production profile-closure snapshot transcript drifted".to_owned());
    }
    Ok(())
}

#[cfg(all(test, feature = "repository-input-candidate"))]
mod tests {
    use super::*;

    fn managed_specification() -> ProfileClosureSpecification {
        super::super::ProductionSpecification::parse(include_bytes!(
            "../../../../../docs/conformance/u4c-metrics.toml"
        ))
        .expect("managed U4c production specification must parse")
        .profile_closure
    }

    fn managed_measurement() -> super::super::MeasurementSpecification {
        super::super::ProductionSpecification::parse(include_bytes!(
            "../../../../../docs/conformance/u4c-metrics.toml"
        ))
        .expect("managed U4c production specification must parse")
        .measurement
    }

    fn managed_ledger_contract() -> super::super::LedgerContractSpecification {
        super::super::ProductionSpecification::parse(include_bytes!(
            "../../../../../docs/conformance/u4c-metrics.toml"
        ))
        .expect("managed U4c production specification must parse")
        .ledger_contract
    }

    fn assert_specification_rejected(specification: &ProfileClosureSpecification) {
        assert!(
            validate_specification(specification).is_err(),
            "synthetic profile-closure specification drift must fail closed"
        );
    }

    fn assert_common_snapshot_total_drift_rejected(
        mutate: impl Fn(&mut super::super::CanonicalSnapshotTotals),
    ) {
        let mut specification = managed_specification();
        let mut report_totals = managed_specification().snapshot_totals;
        mutate(&mut specification.snapshot_totals);
        mutate(&mut report_totals);
        assert_eq!(report_totals, specification.snapshot_totals);
        assert_specification_rejected(&specification);
    }

    fn matching_shape(
        expected: &ProfileClosureCanonicalShape,
    ) -> ProfileClosureCanonicalShapeReport {
        ProfileClosureCanonicalShapeReport {
            node_count: expected.node_count,
            root_count: expected.root_count,
            camera_count: expected.camera_count,
            light_count: expected.light_count,
            camera_attachment_count: expected.camera_attachment_count,
            light_attachment_count: expected.light_attachment_count,
            unit_scale_to_meters: expected.unit_scale_to_meters,
            profile_resource_count: expected.profile_resource_count,
            profile_source_count: expected.profile_source_count,
            profile_provenance_count: expected.profile_provenance_count,
            profile_owner_count: expected.profile_owner_count,
            profile_attachment_count: expected.profile_attachment_count,
            profile_metadata_entry_count: expected.profile_metadata_entry_count,
        }
    }

    fn matching_mutations(
        expected: &[ProfileClosureMutation],
    ) -> Vec<ProfileClosureMutationReport> {
        expected
            .iter()
            .map(|mutation| ProfileClosureMutationReport {
                field_class: mutation.field_class.clone(),
                snapshot_digest_hex: mutation.snapshot_digest_hex.clone(),
            })
            .collect()
    }

    fn matching_negative_cases(
        expected: &[ProfileClosureNegativeCase],
    ) -> Vec<ProfileClosureNegativeCaseReport> {
        expected
            .iter()
            .map(|case| ProfileClosureNegativeCaseReport {
                scenario_id: case.scenario_id.clone(),
                preflight_disposition: case.preflight_disposition.clone(),
                activation_eligibility_reached: case.activation_eligibility_reached,
                failure_class: case.failure_class.clone(),
                failure_kind: case.failure_kind.clone(),
                failure_code: case.failure_code,
                peak_retained_bytes_delta: case.peak_retained_bytes_delta,
                peak_retained_allocations_delta: case.peak_retained_allocations_delta,
                total_admitted_retained_bytes_delta: case.total_admitted_retained_bytes_delta,
                total_admitted_retained_allocations_delta: case
                    .total_admitted_retained_allocations_delta,
                live_retained_bytes_delta: case.live_retained_bytes_delta,
                live_retained_allocations_delta: case.live_retained_allocations_delta,
                final_live_retained_bytes: case.final_live_retained_bytes,
                final_live_retained_allocations: case.final_live_retained_allocations,
            })
            .collect()
    }

    fn assert_negative_case_report_rejected(
        expected: &[ProfileClosureNegativeCase],
        mutate: impl FnOnce(&mut ProfileClosureNegativeCaseReport),
    ) {
        let mut report = matching_negative_cases(expected);
        mutate(&mut report[0]);
        assert!(
            !negative_case_evidence_matches(&report, expected),
            "synthetic profile-closure negative-case history drift must fail closed"
        );
    }

    fn assert_negative_case_specification_rejected(
        mutate: impl FnOnce(&mut ProfileClosureNegativeCase),
    ) {
        let mut specification = managed_specification();
        mutate(&mut specification.negative_cases[0]);
        assert_specification_rejected(&specification);
    }

    fn different_digest(digest: &str) -> String {
        let mut drifted = digest.to_owned();
        let first = if digest.starts_with('0') { "1" } else { "0" };
        drifted.replace_range(..1, first);
        drifted
    }

    fn retained_account(
        expected: &ProfileClosureSpecification,
    ) -> super::super::PhysicalAccountReport {
        super::super::PhysicalAccountReport {
            live_retained_bytes: expected.retained_bytes,
            live_retained_allocations: expected.retained_allocations,
            peak_retained_bytes: expected.retained_bytes,
            peak_retained_allocations: expected.retained_allocations,
            total_admitted_retained_bytes: expected.retained_bytes,
            total_admitted_retained_allocations: expected.retained_allocations,
            ..super::super::PhysicalAccountReport::default()
        }
    }

    fn matching_physical_lifecycle(
        expected: &ProfileClosureSpecification,
    ) -> PhysicalLifecycleReport {
        PhysicalLifecycleReport {
            baseline: super::super::PhysicalAccountReport::default(),
            retained: retained_account(expected),
            after_inspection: retained_account(expected),
            after_snapshot: retained_account(expected),
            after_release: super::super::PhysicalAccountReport::default(),
        }
    }

    fn empty_taxonomy() -> super::super::MeasurementTaxonomyReport {
        super::super::MeasurementTaxonomyReport {
            semantic_cell_writes: 0,
            reference_writes: 0,
            producer_events: 0,
            payload_copy_bytes: 0,
            payload_backing_allocations: 0,
            block_begins: 0,
            block_seals: 0,
            slot_publications: 0,
            new_block_begins: 0,
            new_block_seals: 0,
            new_slot_publications: 0,
            inherited_block_begins: 0,
            inherited_block_seals: 0,
            inherited_slot_publications: 0,
            element_decodes: 0,
            completed_lookups: 0,
            rejected_lookups: 0,
            lookup_events: 0,
            max_lookup_events: 0,
            member_probes: 0,
            max_member_probes_per_lookup: 0,
        }
    }

    fn matching_inspection_phase(
        expected: &ProfileClosureSpecification,
        measurement: &super::super::MeasurementSpecification,
    ) -> super::super::PhaseReport {
        super::super::PhaseReport {
            scenario_id: expected.inspection_scenario_id.clone(),
            taxonomy: super::super::MeasurementTaxonomyReport {
                completed_lookups: expected.inspection_completed_lookups,
                lookup_events: expected.inspection_lookup_events,
                max_lookup_events: measurement.max_lookup_events,
                member_probes: expected.inspection_member_probes,
                max_member_probes_per_lookup: measurement.max_member_probes,
                ..empty_taxonomy()
            },
            allocations: super::super::AllocationReport::default(),
        }
    }

    fn matching_snapshot_phase(
        expected: &ProfileClosureSpecification,
    ) -> super::super::PhaseReport {
        let budget = &expected.snapshot_budget;
        let allocations = budget.allocations;
        super::super::PhaseReport {
            scenario_id: expected.snapshot_scenario_id.clone(),
            taxonomy: super::super::MeasurementTaxonomyReport {
                completed_lookups: budget.completed_lookups,
                lookup_events: budget.lookup_events,
                max_lookup_events: budget.max_lookup_events,
                member_probes: budget.member_probes,
                max_member_probes_per_lookup: budget.max_member_probes_per_lookup,
                ..empty_taxonomy()
            },
            allocations: super::super::AllocationReport {
                count_total: allocations.max_count_total,
                count_current: i64::try_from(allocations.max_count_current)
                    .expect("managed current allocation count must fit i64"),
                count_max: allocations.max_count_peak,
                bytes_total: allocations.max_bytes_total,
                bytes_current: i64::try_from(allocations.max_bytes_current)
                    .expect("managed current allocation bytes must fit i64"),
                bytes_max: allocations.max_bytes_peak,
            },
        }
    }

    fn matching_construction_phase(
        expected: &ProfileClosureSpecification,
    ) -> super::super::PhaseReport {
        let allocations = expected.construction_budget.allocations;
        super::super::PhaseReport {
            scenario_id: expected.construction_scenario_id.clone(),
            taxonomy: super::super::MeasurementTaxonomyReport {
                semantic_cell_writes: expected.construction_semantic_cells,
                reference_writes: expected.construction_references,
                producer_events: expected.construction_producer_events,
                payload_copy_bytes: expected.construction_budget.max_payload_copy_bytes,
                payload_backing_allocations: expected.construction_blocks,
                block_begins: expected.construction_blocks,
                block_seals: expected.construction_blocks,
                slot_publications: expected.construction_blocks,
                new_block_begins: expected.construction_blocks,
                new_block_seals: expected.construction_blocks,
                new_slot_publications: expected.construction_blocks,
                inherited_block_begins: 0,
                inherited_block_seals: 0,
                inherited_slot_publications: 0,
                element_decodes: 0,
                completed_lookups: 0,
                rejected_lookups: 0,
                lookup_events: 0,
                max_lookup_events: 0,
                member_probes: 0,
                max_member_probes_per_lookup: 0,
            },
            allocations: super::super::AllocationReport {
                count_total: allocations.max_count_total,
                count_current: i64::try_from(allocations.max_count_current)
                    .expect("managed current allocation count must fit i64"),
                count_max: allocations.max_count_peak,
                bytes_total: allocations.max_bytes_total,
                bytes_current: i64::try_from(allocations.max_bytes_current)
                    .expect("managed current allocation bytes must fit i64"),
                bytes_max: allocations.max_bytes_peak,
            },
        }
    }

    fn matching_success_evidence(
        contract: &super::super::LedgerContractSpecification,
        canonical_block_count: u64,
        retained_evidence_block_count: u64,
    ) -> super::super::EvidenceReport {
        let published_outcome_evidence = retained_evidence_block_count
            .checked_add(1)
            .expect("managed retained-evidence count must have headroom");
        super::super::EvidenceReport {
            retained_evidence_block_count,
            ledger_header: super::super::LedgerHeaderReport {
                schema_revision: 1,
                terminal_cause: super::super::TerminalCauseReport {
                    class: "success".to_owned(),
                    failure_class: None,
                    code: None,
                },
                dimension_count: 2,
                phase_count: 1,
                current_live_bytes: 0,
                peak_live_bytes: 1,
                active_reservations: 0,
                current_evidence_bytes: 0,
                peak_evidence_bytes: 1,
            },
            dimensions: vec![
                super::super::LedgerDimensionReport {
                    role: super::super::LedgerDimensionRole::Ordinary,
                    semantic_id_hex: contract.ordinary_semantic_id_hex.clone(),
                    attempted: 1,
                    consumed: 1,
                    current_live_bytes: 0,
                    peak_live_bytes: 1,
                    pending_publication: 0,
                    committed: canonical_block_count,
                    committed_canonical_output: canonical_block_count,
                    published_outcome_evidence,
                },
                super::super::LedgerDimensionReport {
                    role: super::super::LedgerDimensionRole::FailureTerminal,
                    semantic_id_hex: contract.failure_terminal_semantic_id_hex.clone(),
                    attempted: 0,
                    consumed: 0,
                    current_live_bytes: 0,
                    peak_live_bytes: 0,
                    pending_publication: 0,
                    committed: 0,
                    committed_canonical_output: 0,
                    published_outcome_evidence: 0,
                },
            ],
        }
    }

    #[test]
    fn rejects_attachment_and_provenance_shape_drift() {
        let specification = managed_specification();
        let matching = matching_shape(&specification.canonical_shape);
        assert!(canonical_shape_matches(
            &matching,
            &specification.canonical_shape
        ));

        let mut attachment_drift = matching_shape(&specification.canonical_shape);
        attachment_drift.camera_attachment_count += 1;
        assert!(!canonical_shape_matches(
            &attachment_drift,
            &specification.canonical_shape
        ));

        let mut provenance_drift = matching_shape(&specification.canonical_shape);
        provenance_drift.profile_provenance_count += 1;
        assert!(!canonical_shape_matches(
            &provenance_drift,
            &specification.canonical_shape
        ));

        let mut scale_drift = matching_shape(&specification.canonical_shape);
        scale_drift.unit_scale_to_meters = f64::from_bits(
            specification
                .canonical_shape
                .unit_scale_to_meters
                .to_bits()
                .checked_add(1)
                .expect("managed unit scale must have an adjacent representation"),
        );
        assert!(!canonical_shape_matches(
            &scale_drift,
            &specification.canonical_shape
        ));

        let mut specification_scale_drift = managed_specification();
        specification_scale_drift
            .canonical_shape
            .unit_scale_to_meters = 0.0;
        assert_specification_rejected(&specification_scale_drift);
    }

    #[test]
    fn rejects_metadata_mutation_field_and_digest_drift() {
        let specification = managed_specification();
        let matching = matching_mutations(&specification.mutations);
        assert!(mutation_evidence_matches(
            &matching,
            &specification.mutations
        ));

        let mut field_drift = matching_mutations(&specification.mutations);
        field_drift[0].field_class = "metadata_bytes_value".to_owned();
        assert!(!mutation_evidence_matches(
            &field_drift,
            &specification.mutations
        ));

        let mut digest_drift = matching_mutations(&specification.mutations);
        digest_drift[0].snapshot_digest_hex =
            different_digest(&digest_drift[0].snapshot_digest_hex);
        assert!(!mutation_evidence_matches(
            &digest_drift,
            &specification.mutations
        ));

        let mut unchanged_digest = managed_specification();
        unchanged_digest.mutations[0].snapshot_digest_hex =
            unchanged_digest.full_snapshot_digest_hex.clone();
        assert_specification_rejected(&unchanged_digest);
    }

    #[test]
    fn rejects_snapshot_scope_and_total_invariant_drift() {
        assert_common_snapshot_total_drift_rejected(|totals| totals.image_count = 1);
        assert_common_snapshot_total_drift_rejected(|totals| totals.primitive_count = 1);
        assert_common_snapshot_total_drift_rejected(|totals| totals.skin_count = 1);
        assert_common_snapshot_total_drift_rejected(|totals| totals.animation_clip_count = 1);
        assert_common_snapshot_total_drift_rejected(|totals| {
            totals.profile_wide_value_count = 0;
        });
        assert_common_snapshot_total_drift_rejected(|totals| {
            totals.profile_lineage_text_bytes = 0;
        });
        assert_common_snapshot_total_drift_rejected(|totals| {
            totals.profile_metadata_text_bytes = 0;
        });
        assert_common_snapshot_total_drift_rejected(|totals| {
            totals.profile_metadata_bool_scalar_count = 0;
            totals.profile_metadata_u64_scalar_count = 0;
        });
        assert_common_snapshot_total_drift_rejected(|totals| totals.transcript_bytes = 0);
        assert_common_snapshot_total_drift_rejected(|totals| totals.work_units = 0);
    }

    #[test]
    fn rejects_publication_block_sum_drift_and_overflow() {
        let mut sum_drift = managed_specification();
        sum_drift.construction_blocks = sum_drift
            .construction_blocks
            .checked_add(1)
            .expect("managed construction block count must have headroom");
        assert_specification_rejected(&sum_drift);

        let mut overflow = managed_specification();
        overflow.canonical_block_count = u64::MAX;
        overflow.outcome_evidence_block_count = 1;
        assert_specification_rejected(&overflow);
    }

    #[test]
    fn rejects_synchronized_evidence_and_ledger_drift_from_top_level_count() {
        let specification = managed_specification();
        let contract = managed_ledger_contract();
        let matching = matching_success_evidence(
            &contract,
            specification.canonical_block_count,
            specification.outcome_evidence_block_count,
        );
        assert!(
            super::super::validate_success_evidence(
                &contract,
                &matching,
                specification.canonical_block_count,
                "managed profile-closure evidence",
            )
            .is_ok()
        );
        assert!(outcome_evidence_count_matches(
            matching.retained_evidence_block_count,
            specification.outcome_evidence_block_count
        ));

        let drifted_retained_evidence_block_count = specification
            .outcome_evidence_block_count
            .checked_add(1)
            .expect("managed outcome-evidence count must have headroom");
        let synchronized_drift = matching_success_evidence(
            &contract,
            specification.canonical_block_count,
            drifted_retained_evidence_block_count,
        );
        assert!(
            super::super::validate_success_evidence(
                &contract,
                &synchronized_drift,
                specification.canonical_block_count,
                "synchronized profile-closure evidence drift",
            )
            .is_ok(),
            "the lower evidence and ledger layers remain internally consistent"
        );
        assert!(!outcome_evidence_count_matches(
            synchronized_drift.retained_evidence_block_count,
            specification.outcome_evidence_block_count
        ));
    }

    #[test]
    fn rejects_negative_case_preflight_cause_and_storage_drift() {
        let specification = managed_specification();
        let matching = matching_negative_cases(&specification.negative_cases);
        assert!(negative_case_evidence_matches(
            &matching,
            &specification.negative_cases
        ));

        assert_negative_case_report_rejected(&specification.negative_cases, |case| {
            case.preflight_disposition = "accepted".to_owned();
        });
        assert_negative_case_report_rejected(&specification.negative_cases, |case| {
            case.activation_eligibility_reached = true;
        });

        let mut cause_drift = matching_negative_cases(&specification.negative_cases);
        cause_drift[2].failure_kind = "owner-provenance-out-of-range".to_owned();
        assert!(!negative_case_evidence_matches(
            &cause_drift,
            &specification.negative_cases
        ));

        assert_negative_case_report_rejected(&specification.negative_cases, |case| {
            case.peak_retained_bytes_delta = 1;
        });
        assert_negative_case_report_rejected(&specification.negative_cases, |case| {
            case.peak_retained_allocations_delta = 1;
        });
        assert_negative_case_report_rejected(&specification.negative_cases, |case| {
            case.total_admitted_retained_bytes_delta = 1;
        });
        assert_negative_case_report_rejected(&specification.negative_cases, |case| {
            case.total_admitted_retained_allocations_delta = 1;
        });
        assert_negative_case_report_rejected(&specification.negative_cases, |case| {
            case.live_retained_bytes_delta = 1;
        });
        assert_negative_case_report_rejected(&specification.negative_cases, |case| {
            case.live_retained_allocations_delta = 1;
        });
        assert_negative_case_report_rejected(&specification.negative_cases, |case| {
            case.final_live_retained_bytes = 1;
        });
        assert_negative_case_report_rejected(&specification.negative_cases, |case| {
            case.final_live_retained_allocations = 1;
        });

        assert_negative_case_specification_rejected(|case| {
            case.preflight_disposition = "accepted".to_owned();
        });
        assert_negative_case_specification_rejected(|case| {
            case.activation_eligibility_reached = true;
        });
        assert_negative_case_specification_rejected(|case| {
            case.peak_retained_bytes_delta = 1;
        });
        assert_negative_case_specification_rejected(|case| {
            case.peak_retained_allocations_delta = 1;
        });
        assert_negative_case_specification_rejected(|case| {
            case.total_admitted_retained_bytes_delta = 1;
        });
        assert_negative_case_specification_rejected(|case| {
            case.total_admitted_retained_allocations_delta = 1;
        });
        assert_negative_case_specification_rejected(|case| {
            case.live_retained_bytes_delta = 1;
        });
        assert_negative_case_specification_rejected(|case| {
            case.live_retained_allocations_delta = 1;
        });
        assert_negative_case_specification_rejected(|case| {
            case.final_live_retained_bytes = 1;
        });
        assert_negative_case_specification_rejected(|case| {
            case.final_live_retained_allocations = 1;
        });
    }

    #[test]
    fn rejects_inspection_allocation_rejection_and_envelope_drift() {
        let specification = managed_specification();
        let measurement = managed_measurement();
        let matching = matching_inspection_phase(&specification, &measurement);
        assert!(
            validate_inspection_phase(&matching, &specification, &measurement).is_ok(),
            "managed inspection vector must satisfy the shared hot-view contract"
        );

        let mut allocation_drift = matching_inspection_phase(&specification, &measurement);
        allocation_drift.allocations.count_total = 1;
        allocation_drift.allocations.count_max = 1;
        assert!(
            validate_inspection_phase(&allocation_drift, &specification, &measurement).is_err()
        );

        let mut rejected_lookup = matching_inspection_phase(&specification, &measurement);
        rejected_lookup.taxonomy.rejected_lookups = 1;
        assert!(validate_inspection_phase(&rejected_lookup, &specification, &measurement).is_err());

        let mut envelope_specification = managed_specification();
        envelope_specification.inspection_lookup_events = envelope_specification
            .inspection_completed_lookups
            .checked_mul(measurement.max_lookup_events)
            .and_then(|maximum| maximum.checked_add(1))
            .expect("managed inspection envelope must have headroom");
        let envelope_drift = matching_inspection_phase(&envelope_specification, &measurement);
        assert!(
            validate_inspection_phase(&envelope_drift, &envelope_specification, &measurement)
                .is_err()
        );
    }

    #[test]
    fn rejects_nested_snapshot_count_and_rejection_drift() {
        let specification = managed_specification();
        let matching = matching_snapshot_phase(&specification);
        assert!(
            validate_snapshot_phase(&matching, &specification).is_ok(),
            "managed snapshot vector must match its nested budget"
        );

        let mut count_drift = matching_snapshot_phase(&specification);
        count_drift.taxonomy.completed_lookups += 1;
        assert!(validate_snapshot_phase(&count_drift, &specification).is_err());

        let mut lookup_drift = matching_snapshot_phase(&specification);
        lookup_drift.taxonomy.lookup_events = lookup_drift
            .taxonomy
            .lookup_events
            .checked_sub(1)
            .expect("managed snapshot lookup events must be positive");
        assert!(validate_snapshot_phase(&lookup_drift, &specification).is_err());

        let mut probe_drift = matching_snapshot_phase(&specification);
        probe_drift.taxonomy.member_probes = probe_drift
            .taxonomy
            .member_probes
            .checked_sub(1)
            .expect("managed snapshot member probes must be positive");
        assert!(validate_snapshot_phase(&probe_drift, &specification).is_err());

        let mut rejected_lookup = matching_snapshot_phase(&specification);
        rejected_lookup.taxonomy.rejected_lookups = 1;
        assert!(validate_snapshot_phase(&rejected_lookup, &specification).is_err());

        let mut nested_budget_drift = managed_specification();
        nested_budget_drift.snapshot_budget.completed_lookups = nested_budget_drift
            .snapshot_budget
            .completed_lookups
            .checked_add(1)
            .expect("managed snapshot lookup count must have headroom");
        assert!(
            validate_snapshot_phase(
                &matching_snapshot_phase(&specification),
                &nested_budget_drift
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_budget_and_retained_physical_charge_drift() {
        let mut construction_budget_drift = managed_specification();
        construction_budget_drift
            .construction_budget
            .max_payload_copy_bytes = 0;
        assert_specification_rejected(&construction_budget_drift);

        let mut snapshot_budget_drift = managed_specification();
        snapshot_budget_drift.snapshot_budget.completed_lookups = 0;
        assert_specification_rejected(&snapshot_budget_drift);

        let specification = managed_specification();
        let matching_construction = matching_construction_phase(&specification);
        assert!(construction_phase_matches(
            &matching_construction,
            &specification
        ));

        let mut payload_budget_drift = matching_construction_phase(&specification);
        payload_budget_drift.taxonomy.payload_copy_bytes = specification
            .construction_budget
            .max_payload_copy_bytes
            .checked_add(1)
            .expect("managed payload-copy budget must have headroom");
        assert!(!construction_phase_matches(
            &payload_budget_drift,
            &specification
        ));

        let mut allocation_budget_drift = matching_construction_phase(&specification);
        allocation_budget_drift.allocations.bytes_total = specification
            .construction_budget
            .allocations
            .max_bytes_total
            .checked_add(1)
            .expect("managed allocation budget must have headroom");
        assert!(!construction_phase_matches(
            &allocation_budget_drift,
            &specification
        ));

        let mut lookup_maximum_drift = matching_construction_phase(&specification);
        lookup_maximum_drift.taxonomy.max_lookup_events = 1;
        assert!(!construction_phase_matches(
            &lookup_maximum_drift,
            &specification
        ));

        let mut probe_maximum_drift = matching_construction_phase(&specification);
        probe_maximum_drift.taxonomy.max_member_probes_per_lookup = 1;
        assert!(!construction_phase_matches(
            &probe_maximum_drift,
            &specification
        ));

        let matching = matching_physical_lifecycle(&specification);
        assert!(retained_physical_charge_matches(&matching, &specification));

        let mut retained_charge_drift = matching_physical_lifecycle(&specification);
        retained_charge_drift.retained.live_retained_bytes += 1;
        assert!(!retained_physical_charge_matches(
            &retained_charge_drift,
            &specification
        ));

        let mut lifecycle_drift = matching_physical_lifecycle(&specification);
        lifecycle_drift.after_snapshot.live_retained_allocations += 1;
        assert!(!retained_physical_charge_matches(
            &lifecycle_drift,
            &specification
        ));
    }
}
