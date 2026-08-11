use std::collections::BTreeSet;

use serde::Deserialize;

use super::capability::{
    BoundedPreview as ObjBoundedPreview,
    ConstructionBudgetSpecification as ObjConstructionBudgetSpecification, FixedVector,
    SnapshotBudgetSpecification as ObjSnapshotBudgetSpecification,
};
#[cfg(feature = "repository-input-candidate")]
use super::capability::{
    PhysicalLifecycleReport as ObjPhysicalReport, validate_allocation_lifecycle,
    validate_bounded_preview_report,
    validate_fixed_vector_report as validate_capability_fixed_vector_report,
    validate_nonproducing_phase,
};
use super::{CanonicalSnapshotTotals, CoverageMappingSpecification, is_lowercase_sha256};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ObjCapabilitySpecification {
    entry_point: String,
    scenario_id: String,
    construction_scenario_id: String,
    inspection_scenario_id: String,
    snapshot_scenario_id: String,
    release_scenario_id: String,
    source_shape: ObjSourceShape,
    canonical_shape: ObjCanonicalShape,
    source_vector: FixedVector,
    equivalent_source_vector: FixedVector,
    equivalent_snapshot_digest_hex: String,
    mutations: Vec<ObjMutation>,
    generation_reservations: u64,
    canonical_block_count: u64,
    outcome_evidence_block_count: u64,
    full_snapshot_digest_hex: String,
    snapshot_totals: CanonicalSnapshotTotals,
    bounded_preview: ObjBoundedPreview,
    construction_budget: ObjConstructionBudgetSpecification,
    snapshot_budget: ObjSnapshotBudgetSpecification,
    construction_semantic_cells: u64,
    construction_references: u64,
    construction_blocks: u64,
    construction_producer_events: u64,
    retained_bytes: u64,
    retained_allocations: u64,
}

impl ObjCapabilitySpecification {
    pub(super) fn full_snapshot_digest_hex(&self) -> &str {
        &self.full_snapshot_digest_hex
    }

    pub(super) fn matches_mapping(&self, mapping: &CoverageMappingSpecification) -> bool {
        mapping.entry_point == self.entry_point && mapping.scenario_id == self.scenario_id
    }
}

#[cfg(feature = "repository-input-candidate")]
impl ObjCapabilityReport {
    pub(super) fn full_snapshot_digest_hex(&self) -> &str {
        &self.full_snapshot_digest_hex
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ObjSourceShape {
    position_count: u64,
    texcoord_count: u64,
    normal_count: u64,
    face_count: u64,
    corner_count: u64,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ObjCanonicalShape {
    topology: String,
    vertex_count: u64,
    corner_count: u64,
    face_count: u64,
    vertex_position_elements: u64,
    corner_normal_elements: u64,
    corner_texcoord_elements: u64,
    face_smoothing_group_elements: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObjMutation {
    field_class: String,
    source_vector: FixedVector,
    snapshot_digest_hex: String,
}

pub(super) fn validate_specification(
    specification: &ObjCapabilitySpecification,
) -> Result<(), String> {
    let totals = &specification.snapshot_totals;
    let preview = &specification.bounded_preview;
    let construction_budget = &specification.construction_budget;
    let snapshot_budget = &specification.snapshot_budget;
    let source_shape = &specification.source_shape;
    let canonical_shape = &specification.canonical_shape;
    let scenario_ids = [
        specification.scenario_id.as_str(),
        specification.construction_scenario_id.as_str(),
        specification.inspection_scenario_id.as_str(),
        specification.snapshot_scenario_id.as_str(),
        specification.release_scenario_id.as_str(),
    ];
    if !is_bounded_identity(&specification.entry_point)
        || scenario_ids
            .iter()
            .any(|scenario_id| !is_bounded_identity(scenario_id))
        || scenario_ids.iter().copied().collect::<BTreeSet<_>>().len() != scenario_ids.len()
        || !source_shape_is_well_formed(source_shape)
        || !canonical_shape_matches_source(source_shape, canonical_shape)
        || specification.generation_reservations == 0
        || specification.canonical_block_count == 0
        || specification.outcome_evidence_block_count == 0
        || !is_lowercase_sha256(&specification.full_snapshot_digest_hex)
        || specification.equivalent_snapshot_digest_hex != specification.full_snapshot_digest_hex
        || !snapshot_totals_match_shape(totals, canonical_shape)
        || !preview.has_common_invariants()
        || !construction_budget.is_well_formed()
        || !snapshot_budget.is_well_formed()
        || specification.construction_semantic_cells == 0
        || specification.construction_references == 0
        || specification.construction_blocks == 0
        || specification
            .canonical_block_count
            .checked_add(specification.outcome_evidence_block_count)
            != Some(specification.construction_blocks)
        || specification.construction_producer_events == 0
        || specification.retained_bytes == 0
        || specification.retained_allocations == 0
    {
        return Err(
            "U4c production OBJ capability specification violates a structural or cross-field invariant"
                .to_owned(),
        );
    }
    validate_fixed_vector(&specification.source_vector)?;
    validate_fixed_vector(&specification.equivalent_source_vector)?;
    if specification.source_vector.schema_id != specification.equivalent_source_vector.schema_id
        || specification.source_vector.sha256 == specification.equivalent_source_vector.sha256
    {
        return Err(
            "U4c production equivalent OBJ source vector must differ before projection".to_owned(),
        );
    }

    if specification.mutations.is_empty() {
        return Err("U4c production OBJ must freeze at least one mutation vector".to_owned());
    }
    let mut mutation_fields = BTreeSet::new();
    let mut mutation_vectors = BTreeSet::new();
    let mut mutation_snapshots = BTreeSet::new();
    for mutation in &specification.mutations {
        validate_fixed_vector(&mutation.source_vector)?;
        if !is_bounded_identity(&mutation.field_class)
            || mutation.source_vector.schema_id != specification.source_vector.schema_id
            || !is_lowercase_sha256(&mutation.snapshot_digest_hex)
            || mutation.snapshot_digest_hex == specification.full_snapshot_digest_hex
            || mutation.source_vector.sha256 == specification.source_vector.sha256
            || mutation.source_vector.sha256 == specification.equivalent_source_vector.sha256
            || !mutation_fields.insert(mutation.field_class.as_str())
            || !mutation_vectors.insert(mutation.source_vector.sha256.as_str())
            || !mutation_snapshots.insert(mutation.snapshot_digest_hex.as_str())
        {
            return Err(format!(
                "U4c production OBJ mutation {} is not independently frozen",
                mutation.field_class
            ));
        }
    }
    Ok(())
}

fn is_bounded_identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

fn source_shape_is_well_formed(source: &ObjSourceShape) -> bool {
    source.position_count > 0
        && source.texcoord_count > 0
        && source.normal_count > 0
        && source.face_count > 0
        && source.face_count.checked_mul(3) == Some(source.corner_count)
        && source.position_count <= source.corner_count
        && source.texcoord_count <= source.corner_count
        && source.normal_count <= source.corner_count
}

fn canonical_shape_matches_source(source: &ObjSourceShape, canonical: &ObjCanonicalShape) -> bool {
    canonical.topology == "triangle_list"
        && canonical.vertex_count == source.position_count
        && canonical.corner_count == source.corner_count
        && canonical.face_count == source.face_count
        && canonical.vertex_position_elements == canonical.vertex_count
        && canonical.corner_normal_elements == canonical.corner_count
        && canonical.corner_texcoord_elements == canonical.corner_count
        && canonical.face_smoothing_group_elements == canonical.face_count
}

fn snapshot_totals_match_shape(
    totals: &CanonicalSnapshotTotals,
    canonical: &ObjCanonicalShape,
) -> bool {
    let Some(position_scalars) = canonical.vertex_position_elements.checked_mul(3) else {
        return false;
    };
    let Some(normal_scalars) = canonical.corner_normal_elements.checked_mul(3) else {
        return false;
    };
    let Some(texcoord_scalars) = canonical.corner_texcoord_elements.checked_mul(2) else {
        return false;
    };
    let Some(f32_scalars) = position_scalars
        .checked_add(normal_scalars)
        .and_then(|value| value.checked_add(texcoord_scalars))
    else {
        return false;
    };
    let u32_scalars = canonical.face_smoothing_group_elements;
    let Some(canonical_scalars) = f32_scalars.checked_add(u32_scalars) else {
        return false;
    };
    let expected_attributes = 1_u64
        + u64::from(canonical.corner_normal_elements > 0)
        + u64::from(canonical.corner_texcoord_elements > 0)
        + u64::from(canonical.face_smoothing_group_elements > 0);

    totals.node_count > 0
        && totals.root_count > 0
        && totals.root_count <= totals.node_count
        && totals.node_count.checked_sub(totals.root_count) == Some(totals.edge_count)
        && totals.has_empty_spatial_domain()
        && totals.logical_mesh_count > 0
        && totals.primitive_count > 0
        && totals.attribute_count == expected_attributes
        && totals.mesh_instance_count > 0
        && totals.primitive_index_count == canonical.corner_count
        && totals.canonical_scalar_count == canonical_scalars
        && totals.f32_scalar_count == f32_scalars
        && totals.u32_scalar_count == u32_scalars
        && totals.primitive_deformation_count == totals.primitive_count
        && totals.deformation_mesh_instance_count == totals.mesh_instance_count
        && totals.profile_owner_count > 0
        && totals.has_fresh_profile(totals.profile_owner_count)
        && totals.transcript_bytes > 0
        && totals.work_units > 0
}

fn validate_fixed_vector(vector: &FixedVector) -> Result<(), String> {
    if !is_bounded_identity(&vector.schema_id)
        || vector.observed_bytes == 0
        || !is_lowercase_sha256(&vector.sha256)
    {
        return Err("U4c production OBJ fixed vector is malformed".to_owned());
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ObjCapabilityReport {
    entry_point: String,
    scenario_id: String,
    source_shape: ObjSourceShape,
    canonical_shape: ObjCanonicalShape,
    source_vector: FixedVector,
    equivalent_source_vector: FixedVector,
    equivalent_snapshot_digest_hex: String,
    mutations: Vec<ObjMutation>,
    generation_reservations: u64,
    canonical_block_count: u64,
    outcome_evidence_block_count: u64,
    full_snapshot_digest_hex: String,
    bounded_snapshot_digest_hex: String,
    snapshot_totals: CanonicalSnapshotTotals,
    bounded_preview: ObjBoundedPreview,
    evidence: super::EvidenceReport,
    construction: super::PhaseReport,
    inspection: super::PhaseReport,
    snapshot: super::PhaseReport,
    release: super::ReleaseReport,
    physical_account: ObjPhysicalReport,
    factory_after_account_release: super::PhysicalFactoryReport,
}

#[cfg(feature = "repository-input-candidate")]
use super::{
    PhaseReport, ProductionSpecification, validate_healthy_account, validate_hot_view_phase,
    validate_producer_bound, validate_release, validate_released_account,
    validate_released_factory, validate_success_evidence,
};

#[cfg(feature = "repository-input-candidate")]
pub(super) fn validate_report(
    specification: &ProductionSpecification,
    report: &ObjCapabilityReport,
) -> Result<(), String> {
    let expected = &specification.obj_capability;
    if report.entry_point != expected.entry_point
        || report.scenario_id != expected.scenario_id
        || report.source_shape != expected.source_shape
        || report.canonical_shape != expected.canonical_shape
        || report.generation_reservations != expected.generation_reservations
        || report.canonical_block_count != expected.canonical_block_count
        || report.outcome_evidence_block_count != expected.outcome_evidence_block_count
        || report.full_snapshot_digest_hex != expected.full_snapshot_digest_hex
        || report.bounded_snapshot_digest_hex != expected.full_snapshot_digest_hex
        || !is_lowercase_sha256(&report.full_snapshot_digest_hex)
    {
        return Err("U4c production OBJ capability identity or digest drifted".to_owned());
    }
    validate_fixed_vector_report(&report.source_vector, &expected.source_vector)?;
    validate_fixed_vector_report(
        &report.equivalent_source_vector,
        &expected.equivalent_source_vector,
    )?;
    if report.equivalent_snapshot_digest_hex != expected.equivalent_snapshot_digest_hex
        || report.source_vector.sha256 == report.equivalent_source_vector.sha256
        || report.mutations.len() != expected.mutations.len()
    {
        return Err("U4c production OBJ equivalent-source evidence drifted".to_owned());
    }
    for (index, (mutation, expected_mutation)) in
        report.mutations.iter().zip(&expected.mutations).enumerate()
    {
        validate_fixed_vector_report(&mutation.source_vector, &expected_mutation.source_vector)?;
        if mutation.field_class != expected_mutation.field_class
            || mutation.snapshot_digest_hex != expected_mutation.snapshot_digest_hex
            || mutation.snapshot_digest_hex == report.full_snapshot_digest_hex
            || report.mutations[..index]
                .iter()
                .any(|earlier| earlier.snapshot_digest_hex == mutation.snapshot_digest_hex)
        {
            return Err(format!(
                "U4c production OBJ mutation {} drifted",
                expected_mutation.field_class
            ));
        }
    }
    validate_snapshot_totals(&report.snapshot_totals, &expected.snapshot_totals)?;
    validate_bounded_preview(&report.bounded_preview, &expected.bounded_preview)?;
    if report.evidence.retained_evidence_block_count != report.outcome_evidence_block_count {
        return Err(
            "U4c production OBJ outcome evidence count disagrees with the ledger".to_owned(),
        );
    }
    validate_success_evidence(
        &specification.ledger_contract,
        &report.evidence,
        expected.canonical_block_count,
        "U4c production OBJ capability",
    )?;
    validate_construction_phase(&report.construction, expected)?;
    if report.inspection.scenario_id != expected.inspection_scenario_id {
        return Err("U4c production OBJ inspection scenario identity drifted".to_owned());
    }
    validate_hot_view_phase(
        &report.inspection,
        &specification.measurement,
        "U4c production OBJ typed inspection",
    )?;
    validate_nonproducing_phase(&report.inspection, "U4c production OBJ typed inspection")?;
    validate_snapshot_phase(&report.snapshot, expected)?;
    if report.release.scenario_id != expected.release_scenario_id {
        return Err("U4c production OBJ release scenario identity drifted".to_owned());
    }
    validate_release(&report.release, "U4c production OBJ release")?;
    validate_allocation_lifecycle(
        [
            &report.construction.allocations,
            &report.inspection.allocations,
            &report.snapshot.allocations,
            &report.release.allocations,
        ],
        "U4c production OBJ allocation lifecycle",
    )?;

    validate_released_account(&report.physical_account.baseline, "U4c OBJ baseline")?;
    validate_healthy_account(&report.physical_account.retained, "U4c OBJ retained")?;
    if report.physical_account.retained.live_retained_bytes != expected.retained_bytes
        || report.physical_account.retained.live_retained_allocations
            != expected.retained_allocations
        || report.physical_account.after_inspection != report.physical_account.retained
        || report.physical_account.after_snapshot != report.physical_account.retained
    {
        return Err("U4c production OBJ retained physical charge drifted".to_owned());
    }
    validate_released_account(
        &report.physical_account.after_release,
        "U4c OBJ final release",
    )?;
    validate_released_factory(
        &report.factory_after_account_release,
        "U4c OBJ factory release",
    )
}

#[cfg(feature = "repository-input-candidate")]
fn validate_fixed_vector_report(
    report: &FixedVector,
    expected: &FixedVector,
) -> Result<(), String> {
    validate_capability_fixed_vector_report(report, expected, "U4c production OBJ")
}

#[cfg(feature = "repository-input-candidate")]
fn validate_snapshot_totals(
    report: &CanonicalSnapshotTotals,
    expected: &CanonicalSnapshotTotals,
) -> Result<(), String> {
    if report != expected {
        return Err("U4c production OBJ snapshot totals drifted".to_owned());
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
fn validate_construction_phase(
    report: &PhaseReport,
    expected: &ObjCapabilitySpecification,
) -> Result<(), String> {
    let taxonomy = &report.taxonomy;
    let allocations = &report.allocations;
    let budget = &expected.construction_budget;
    if report.scenario_id != expected.construction_scenario_id
        || taxonomy.semantic_cell_writes != expected.construction_semantic_cells
        || taxonomy.reference_writes != expected.construction_references
        || taxonomy.block_begins != expected.construction_blocks
        || taxonomy.block_seals != expected.construction_blocks
        || taxonomy.slot_publications != expected.construction_blocks
        || taxonomy.new_block_begins != expected.construction_blocks
        || taxonomy.new_block_seals != expected.construction_blocks
        || taxonomy.new_slot_publications != expected.construction_blocks
        || taxonomy.inherited_block_begins != 0
        || taxonomy.inherited_block_seals != 0
        || taxonomy.inherited_slot_publications != 0
        || taxonomy.producer_events != expected.construction_producer_events
        || taxonomy.payload_copy_bytes > budget.max_payload_copy_bytes
        || taxonomy.payload_backing_allocations != expected.construction_blocks
        || taxonomy.element_decodes != 0
        || taxonomy.completed_lookups != 0
        || taxonomy.rejected_lookups != 0
        || taxonomy.lookup_events != 0
        || taxonomy.max_lookup_events != 0
        || taxonomy.member_probes != 0
        || taxonomy.max_member_probes_per_lookup != 0
        || !budget.allocations.contains(allocations)
    {
        return Err("U4c production OBJ construction transcript or budget drifted".to_owned());
    }
    validate_producer_bound(report, "U4c production OBJ construction")
}

#[cfg(feature = "repository-input-candidate")]
fn validate_snapshot_phase(
    report: &PhaseReport,
    expected: &ObjCapabilitySpecification,
) -> Result<(), String> {
    let taxonomy = &report.taxonomy;
    let allocations = &report.allocations;
    let budget = &expected.snapshot_budget;
    validate_nonproducing_phase(report, "U4c production OBJ snapshot")?;
    if report.scenario_id != expected.snapshot_scenario_id
        || taxonomy.completed_lookups != budget.completed_lookups
        || taxonomy.rejected_lookups != 0
        || taxonomy.lookup_events != budget.lookup_events
        || taxonomy.max_lookup_events != budget.max_lookup_events
        || taxonomy.member_probes != budget.member_probes
        || taxonomy.max_member_probes_per_lookup != budget.max_member_probes_per_lookup
        || !budget.allocations.contains(allocations)
    {
        return Err("U4c production OBJ snapshot transcript or budget drifted".to_owned());
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
fn validate_bounded_preview(
    report: &ObjBoundedPreview,
    expected: &ObjBoundedPreview,
) -> Result<(), String> {
    validate_bounded_preview_report(report, expected, "U4c production OBJ")
}

#[cfg(all(test, feature = "repository-input-candidate"))]
mod tests {
    use super::*;

    fn managed_specification() -> ObjCapabilitySpecification {
        let specification = super::super::ProductionSpecification::parse(include_bytes!(
            "../../../../../docs/conformance/u4c-metrics.toml"
        ))
        .expect("managed U4c production specification must parse");
        specification.obj_capability
    }

    #[test]
    fn rejects_profile_and_spatial_snapshot_drift() {
        let specification = managed_specification();
        validate_specification(&specification)
            .expect("managed OBJ capability specification must remain valid");

        let mut profile_drift = managed_specification();
        profile_drift.snapshot_totals.profile_attachment_count -= 1;
        assert!(validate_specification(&profile_drift).is_err());

        let mut spatial_drift = managed_specification();
        spatial_drift.snapshot_totals.light_count += 1;
        assert!(validate_specification(&spatial_drift).is_err());
    }

    #[test]
    fn rejects_same_length_bounded_preview_drift() {
        let expected = ObjBoundedPreview {
            schema_id: "baozi.canonical-scene.bounded-preview.v1".to_owned(),
            bytes: 8,
            sha256: "1111111111111111111111111111111111111111111111111111111111111111".to_owned(),
            items_omitted_by_limit: 1,
            bytes_truncated: true,
            items_truncated: true,
        };
        let report = ObjBoundedPreview {
            schema_id: expected.schema_id.clone(),
            bytes: expected.bytes,
            sha256: "2222222222222222222222222222222222222222222222222222222222222222".to_owned(),
            items_omitted_by_limit: expected.items_omitted_by_limit,
            bytes_truncated: expected.bytes_truncated,
            items_truncated: expected.items_truncated,
        };

        assert!(validate_bounded_preview(&report, &expected).is_err());
    }
}
