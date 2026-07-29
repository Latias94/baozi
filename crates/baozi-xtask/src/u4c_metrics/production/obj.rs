use serde::Deserialize;

use super::capability::{
    BoundedPreview as ObjBoundedPreview,
    ConstructionBudgetSpecification as ObjConstructionBudgetSpecification, FixedVector,
    SnapshotBudgetSpecification as ObjSnapshotBudgetSpecification,
    validate_fixed_vector as validate_capability_fixed_vector,
};
#[cfg(feature = "repository-input-candidate")]
use super::capability::{
    PhysicalLifecycleReport as ObjPhysicalReport, validate_allocation_lifecycle,
    validate_bounded_preview_report,
    validate_fixed_vector_report as validate_capability_fixed_vector_report,
    validate_nonproducing_phase,
};
use super::{CoverageMappingSpecification, is_lowercase_sha256, validate_exact_list};

const SOURCE_VECTOR_SCHEMA_ID: &str = "baozi.normalized-obj-capability-source.v1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ObjCapabilitySpecification {
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
    snapshot_totals: ObjSnapshotTotals,
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
    pub(super) fn matches_mapping(&self, mapping: &CoverageMappingSpecification) -> bool {
        mapping.entry_point == self.entry_point && mapping.scenario_id == self.scenario_id
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

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ObjSnapshotTotals {
    node_count: u64,
    root_count: u64,
    edge_count: u64,
    material_count: u64,
    logical_mesh_count: u64,
    primitive_count: u64,
    attribute_count: u64,
    mesh_instance_count: u64,
    polygon_boundary_count: u64,
    primitive_index_count: u64,
    sparse_index_count: u64,
    canonical_scalar_count: u64,
    f32_scalar_count: u64,
    u32_scalar_count: u64,
    transcript_bytes: u64,
    work_units: u64,
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
    if specification.entry_point != "canonical-obj-corner-smoothing"
        || specification.scenario_id != "canonical-obj-corner-smoothing-v1"
        || source_shape.position_count != 3
        || source_shape.texcoord_count != 4
        || source_shape.normal_count != 2
        || source_shape.face_count != 2
        || source_shape.corner_count != 6
        || source_shape.face_count.checked_mul(3) != Some(source_shape.corner_count)
        || canonical_shape.topology != "triangle_list"
        || canonical_shape.vertex_count != 3
        || canonical_shape.corner_count != 6
        || canonical_shape.face_count != 2
        || canonical_shape.vertex_position_elements != 3
        || canonical_shape.corner_normal_elements != 6
        || canonical_shape.corner_texcoord_elements != 6
        || canonical_shape.face_smoothing_group_elements != 2
        || specification.generation_reservations != 1
        || specification.canonical_block_count == 0
        || specification.outcome_evidence_block_count == 0
        || !is_lowercase_sha256(&specification.full_snapshot_digest_hex)
        || specification.equivalent_snapshot_digest_hex != specification.full_snapshot_digest_hex
        || totals.node_count != 1
        || totals.root_count != 1
        || totals.edge_count != 0
        || totals.material_count != 0
        || totals.logical_mesh_count != 1
        || totals.primitive_count != 1
        || totals.attribute_count != 4
        || totals.mesh_instance_count != 1
        || totals.polygon_boundary_count != 0
        || totals.primitive_index_count != 6
        || totals.sparse_index_count != 0
        || totals.canonical_scalar_count != 41
        || totals.f32_scalar_count != 39
        || totals.u32_scalar_count != 2
        || totals.transcript_bytes == 0
        || totals.work_units == 0
        || !preview.has_common_invariants()
        || !preview.bytes_truncated
        || !preview.items_truncated
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
            "U4c production OBJ capability specification is not the frozen canonical vector"
                .to_owned(),
        );
    }
    validate_fixed_vector(&specification.source_vector)?;
    validate_fixed_vector(&specification.equivalent_source_vector)?;
    if specification.source_vector.sha256 == specification.equivalent_source_vector.sha256 {
        return Err(
            "U4c production equivalent OBJ source vector must differ before projection".to_owned(),
        );
    }

    let expected_mutations = [
        "position_index",
        "texcoord_index",
        "normal_index",
        "smoothing_group",
    ];
    let actual_mutations = specification
        .mutations
        .iter()
        .map(|mutation| mutation.field_class.as_str())
        .collect::<Vec<_>>();
    validate_exact_list(
        "obj_capability.mutations",
        &actual_mutations,
        &expected_mutations,
    )?;
    for (index, mutation) in specification.mutations.iter().enumerate() {
        validate_fixed_vector(&mutation.source_vector)?;
        if !is_lowercase_sha256(&mutation.snapshot_digest_hex)
            || mutation.snapshot_digest_hex == specification.full_snapshot_digest_hex
            || mutation.source_vector.sha256 == specification.source_vector.sha256
            || mutation.source_vector.sha256 == specification.equivalent_source_vector.sha256
            || specification.mutations[..index].iter().any(|earlier| {
                earlier.snapshot_digest_hex == mutation.snapshot_digest_hex
                    || earlier.source_vector.sha256 == mutation.source_vector.sha256
            })
        {
            return Err(format!(
                "U4c production OBJ mutation {} is not independently frozen",
                mutation.field_class
            ));
        }
    }
    Ok(())
}

fn validate_fixed_vector(vector: &FixedVector) -> Result<(), String> {
    validate_capability_fixed_vector(vector, SOURCE_VECTOR_SCHEMA_ID, "U4c production OBJ")
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
    snapshot_totals: ObjSnapshotTotals,
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
    if report.evidence.outcome_evidence_block_count != report.outcome_evidence_block_count {
        return Err(
            "U4c production OBJ outcome evidence count disagrees with the ledger".to_owned(),
        );
    }
    validate_success_evidence(&report.evidence, "U4c production OBJ capability")?;
    validate_construction_phase(&report.construction, expected)?;
    if report.inspection.scenario_id != "canonical-obj-corner-smoothing-inspection-v1" {
        return Err("U4c production OBJ inspection scenario identity drifted".to_owned());
    }
    validate_hot_view_phase(
        &report.inspection,
        &specification.measurement,
        "U4c production OBJ typed inspection",
    )?;
    validate_nonproducing_phase(&report.inspection, "U4c production OBJ typed inspection")?;
    validate_snapshot_phase(&report.snapshot, expected)?;
    if report.release.scenario_id != "canonical-obj-corner-smoothing-release-v1" {
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
    report: &ObjSnapshotTotals,
    expected: &ObjSnapshotTotals,
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
    if report.scenario_id != "canonical-obj-corner-smoothing-construction-v1"
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
    if report.scenario_id != "canonical-obj-corner-smoothing-snapshot-v1"
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
