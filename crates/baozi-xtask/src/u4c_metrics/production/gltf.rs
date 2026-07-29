use serde::Deserialize;

use super::capability::{
    BoundedPreview as GltfBoundedPreview, ConstructionBudgetSpecification, FixedVector,
    SnapshotBudgetSpecification, validate_fixed_vector as validate_capability_fixed_vector,
};
#[cfg(feature = "repository-input-candidate")]
use super::capability::{
    PhysicalLifecycleReport as GltfPhysicalReport, validate_allocation_lifecycle,
    validate_bounded_preview_report,
    validate_fixed_vector_report as validate_capability_fixed_vector_report,
    validate_nonproducing_phase,
};
use super::{CoverageMappingSpecification, is_lowercase_sha256, validate_exact_list};

const SOURCE_VECTOR_SCHEMA_ID: &str = "baozi.selected-gltf-shape-capability-source.v1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GltfCapabilitySpecification {
    entry_point: String,
    scenario_id: String,
    source_shape: GltfSourceShape,
    canonical_shape: GltfCanonicalShape,
    source_vector: FixedVector,
    mutations: Vec<GltfMutation>,
    generation_reservations: u64,
    canonical_block_count: u64,
    outcome_evidence_block_count: u64,
    full_snapshot_digest_hex: String,
    snapshot_totals: GltfSnapshotTotals,
    bounded_preview: GltfBoundedPreview,
    construction_budget: ConstructionBudgetSpecification,
    snapshot_budget: SnapshotBudgetSpecification,
    construction_semantic_cells: u64,
    construction_references: u64,
    construction_blocks: u64,
    construction_producer_events: u64,
    retained_bytes: u64,
    retained_allocations: u64,
}

impl GltfCapabilitySpecification {
    pub(super) fn matches_mapping(&self, mapping: &CoverageMappingSpecification) -> bool {
        mapping.entry_point == self.entry_point && mapping.scenario_id == self.scenario_id
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct GltfSourceShape {
    node_count: u64,
    root_count: u64,
    image_count: u64,
    image_payload_bytes: u64,
    media_type_bytes: u64,
    sampler_count: u64,
    texture_count: u64,
    material_count: u64,
    material_binding_count: u64,
    logical_mesh_count: u64,
    primitive_count: u64,
    attribute_count: u64,
    mesh_instance_count: u64,
    skin_count: u64,
    skin_joint_count: u64,
    morph_definition_count: u64,
    morph_target_count: u64,
    morph_delta_count: u64,
    influence_row_count: u64,
    influence_count: u64,
    animation_clip_count: u64,
    animation_channel_count: u64,
    animation_key_count: u64,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct GltfCanonicalShape {
    node_count: u64,
    root_count: u64,
    edge_count: u64,
    image_count: u64,
    sampler_count: u64,
    texture_count: u64,
    material_count: u64,
    material_binding_count: u64,
    logical_mesh_count: u64,
    primitive_count: u64,
    attribute_count: u64,
    mesh_instance_count: u64,
    skin_count: u64,
    skin_joint_count: u64,
    logical_morph_count: u64,
    morph_definition_count: u64,
    primitive_deformation_count: u64,
    morph_target_count: u64,
    morph_delta_count: u64,
    influence_row_count: u64,
    influence_count: u64,
    animation_clip_count: u64,
    animation_channel_count: u64,
    animation_key_count: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GltfMutation {
    field_class: String,
    source_vector: FixedVector,
    snapshot_digest_hex: String,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct GltfSnapshotTotals {
    node_count: u64,
    root_count: u64,
    edge_count: u64,
    image_count: u64,
    sampler_count: u64,
    texture_count: u64,
    material_count: u64,
    material_binding_count: u64,
    image_payload_bytes: u64,
    media_type_bytes: u64,
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
    skin_count: u64,
    logical_morph_count: u64,
    morph_definition_count: u64,
    primitive_deformation_count: u64,
    morph_target_count: u64,
    morph_delta_count: u64,
    deformation_mesh_instance_count: u64,
    skin_joint_count: u64,
    inverse_bind_matrix_count: u64,
    skin_correction_matrix_count: u64,
    skin_name_bytes: u64,
    morph_default_weight_count: u64,
    morph_name_bytes: u64,
    influence_row_count: u64,
    influence_count: u64,
    morph_value_vector_count: u64,
    morph_sparse_index_count: u64,
    instance_morph_weight_count: u64,
    animation_clip_count: u64,
    animation_channel_count: u64,
    animation_time_scalar_count: u64,
    animation_value_scalar_count: u64,
    animation_in_tangent_scalar_count: u64,
    animation_out_tangent_scalar_count: u64,
    transcript_bytes: u64,
    work_units: u64,
}

pub(super) fn validate_specification(
    specification: &GltfCapabilitySpecification,
) -> Result<(), String> {
    let source = &specification.source_shape;
    let canonical = &specification.canonical_shape;
    let totals = &specification.snapshot_totals;
    let preview = &specification.bounded_preview;
    let construction_budget = &specification.construction_budget;
    let snapshot_budget = &specification.snapshot_budget;
    if specification.entry_point != "backend-free-gltf-shape"
        || specification.scenario_id != "backend-free-gltf-shape-v1"
        || source.node_count != 3
        || source.root_count != 2
        || source.image_count != 2
        || source.image_payload_bytes != 20
        || source.media_type_bytes != 19
        || source.sampler_count != 2
        || source.texture_count != 3
        || source.material_count != 2
        || source.material_binding_count != 3
        || source.logical_mesh_count != 1
        || source.primitive_count != 2
        || source.attribute_count != 6
        || source.mesh_instance_count != 1
        || source.skin_count != 1
        || source.skin_joint_count != 2
        || source.morph_definition_count != 2
        || source.morph_target_count != 4
        || source.morph_delta_count != 4
        || source.influence_row_count != 6
        || source.influence_count != 11
        || source.animation_clip_count != 1
        || source.animation_channel_count != 3
        || source.animation_key_count != 6
        || canonical.node_count != source.node_count
        || canonical.root_count != source.root_count
        || canonical.edge_count != 1
        || canonical.image_count != source.image_count
        || canonical.sampler_count != source.sampler_count
        || canonical.texture_count != source.texture_count
        || canonical.material_count != source.material_count
        || canonical.material_binding_count != source.material_binding_count
        || canonical.logical_mesh_count != source.logical_mesh_count
        || canonical.primitive_count != source.primitive_count
        || canonical.attribute_count != source.attribute_count
        || canonical.mesh_instance_count != source.mesh_instance_count
        || canonical.skin_count != source.skin_count
        || canonical.skin_joint_count != source.skin_joint_count
        || canonical.logical_morph_count != 1
        || canonical.morph_definition_count != source.morph_definition_count
        || canonical.primitive_deformation_count != source.primitive_count
        || canonical.morph_target_count != source.morph_target_count
        || canonical.morph_delta_count != source.morph_delta_count
        || canonical.influence_row_count != source.influence_row_count
        || canonical.influence_count != source.influence_count
        || canonical.animation_clip_count != source.animation_clip_count
        || canonical.animation_channel_count != source.animation_channel_count
        || canonical.animation_key_count != source.animation_key_count
        || specification.generation_reservations != 1
        || specification.canonical_block_count == 0
        || specification.outcome_evidence_block_count == 0
        || !is_lowercase_sha256(&specification.full_snapshot_digest_hex)
        || totals.node_count != canonical.node_count
        || totals.root_count != canonical.root_count
        || totals.edge_count != canonical.edge_count
        || totals.image_count != canonical.image_count
        || totals.sampler_count != canonical.sampler_count
        || totals.texture_count != canonical.texture_count
        || totals.material_count != canonical.material_count
        || totals.material_binding_count != canonical.material_binding_count
        || totals.image_payload_bytes != source.image_payload_bytes
        || totals.media_type_bytes != source.media_type_bytes
        || totals.logical_mesh_count != canonical.logical_mesh_count
        || totals.primitive_count != canonical.primitive_count
        || totals.attribute_count != canonical.attribute_count
        || totals.mesh_instance_count != canonical.mesh_instance_count
        || totals.polygon_boundary_count != 0
        || totals.primitive_index_count != 6
        || totals.sparse_index_count != 0
        || totals.canonical_scalar_count != 48
        || totals.f32_scalar_count != 48
        || totals.u32_scalar_count != 0
        || totals.skin_count != canonical.skin_count
        || totals.logical_morph_count != canonical.logical_morph_count
        || totals.morph_definition_count != canonical.morph_definition_count
        || totals.primitive_deformation_count != canonical.primitive_deformation_count
        || totals.morph_target_count != canonical.morph_target_count
        || totals.morph_delta_count != canonical.morph_delta_count
        || totals.deformation_mesh_instance_count != 1
        || totals.skin_joint_count != 2
        || totals.inverse_bind_matrix_count != 2
        || totals.skin_correction_matrix_count != 0
        || totals.skin_name_bytes != 8
        || totals.morph_default_weight_count != 2
        || totals.morph_name_bytes != 10
        || totals.influence_row_count != 6
        || totals.influence_count != 11
        || totals.morph_value_vector_count != 8
        || totals.morph_sparse_index_count != 2
        || totals.instance_morph_weight_count != 2
        || totals.animation_clip_count != 1
        || totals.animation_channel_count != 3
        || totals.animation_time_scalar_count != 6
        || totals.animation_value_scalar_count != 18
        || totals.animation_in_tangent_scalar_count != 4
        || totals.animation_out_tangent_scalar_count != 4
        || totals.transcript_bytes == 0
        || totals.work_units == 0
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
            "U4c production glTF capability specification is not the frozen five-domain vector"
                .to_owned(),
        );
    }
    validate_fixed_vector(&specification.source_vector)?;
    let expected_mutations = [
        "node_transform_float_bits",
        "image_payload_bytes",
        "primitive_material_reference",
        "skin_inverse_bind_float_bits",
        "influence_weight_float_bits",
        "morph_sparse_index",
        "animation_value_float_bits",
    ];
    let actual_mutations = specification
        .mutations
        .iter()
        .map(|mutation| mutation.field_class.as_str())
        .collect::<Vec<_>>();
    validate_exact_list(
        "gltf_capability.mutations",
        &actual_mutations,
        &expected_mutations,
    )?;
    for (index, mutation) in specification.mutations.iter().enumerate() {
        validate_fixed_vector(&mutation.source_vector)?;
        if !is_lowercase_sha256(&mutation.snapshot_digest_hex)
            || mutation.snapshot_digest_hex == specification.full_snapshot_digest_hex
            || mutation.source_vector.sha256 == specification.source_vector.sha256
            || specification.mutations[..index].iter().any(|earlier| {
                earlier.snapshot_digest_hex == mutation.snapshot_digest_hex
                    || earlier.source_vector.sha256 == mutation.source_vector.sha256
            })
        {
            return Err(format!(
                "U4c production glTF mutation {} is not independently frozen",
                mutation.field_class
            ));
        }
    }
    Ok(())
}

fn validate_fixed_vector(vector: &FixedVector) -> Result<(), String> {
    validate_capability_fixed_vector(vector, SOURCE_VECTOR_SCHEMA_ID, "U4c production glTF")
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GltfCapabilityReport {
    entry_point: String,
    scenario_id: String,
    source_shape: GltfSourceShape,
    canonical_shape: GltfCanonicalShape,
    source_vector: FixedVector,
    mutations: Vec<GltfMutation>,
    generation_reservations: u64,
    canonical_block_count: u64,
    outcome_evidence_block_count: u64,
    full_snapshot_digest_hex: String,
    bounded_snapshot_digest_hex: String,
    snapshot_totals: GltfSnapshotTotals,
    bounded_preview: GltfBoundedPreview,
    evidence: super::EvidenceReport,
    construction: super::PhaseReport,
    inspection: super::PhaseReport,
    snapshot: super::PhaseReport,
    release: super::ReleaseReport,
    physical_account: GltfPhysicalReport,
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
    report: &GltfCapabilityReport,
) -> Result<(), String> {
    let expected = &specification.gltf_capability;
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
        || report.snapshot_totals != expected.snapshot_totals
    {
        return Err("U4c production glTF capability identity or snapshot drifted".to_owned());
    }
    validate_bounded_preview_report(
        &report.bounded_preview,
        &expected.bounded_preview,
        "U4c production glTF",
    )?;
    validate_fixed_vector_report(&report.source_vector, &expected.source_vector)?;
    if report.mutations.len() != expected.mutations.len() {
        return Err("U4c production glTF mutation count drifted".to_owned());
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
                "U4c production glTF mutation {} drifted",
                expected_mutation.field_class
            ));
        }
    }
    if report.evidence.outcome_evidence_block_count != report.outcome_evidence_block_count {
        return Err(
            "U4c production glTF outcome evidence count disagrees with the ledger".to_owned(),
        );
    }
    validate_success_evidence(&report.evidence, "U4c production glTF capability")?;
    validate_construction_phase(&report.construction, expected)?;
    if report.inspection.scenario_id != "canonical-gltf-five-domain-inspection-v1" {
        return Err("U4c production glTF inspection scenario identity drifted".to_owned());
    }
    validate_hot_view_phase(
        &report.inspection,
        &specification.measurement,
        "U4c production glTF typed inspection",
    )?;
    validate_nonproducing_phase(&report.inspection, "U4c production glTF typed inspection")?;
    validate_snapshot_phase(&report.snapshot, expected)?;
    if report.release.scenario_id != "canonical-gltf-five-domain-release-v1" {
        return Err("U4c production glTF release scenario identity drifted".to_owned());
    }
    validate_release(&report.release, "U4c production glTF release")?;
    validate_allocation_lifecycle(
        [
            &report.construction.allocations,
            &report.inspection.allocations,
            &report.snapshot.allocations,
            &report.release.allocations,
        ],
        "U4c production glTF allocation lifecycle",
    )?;
    validate_released_account(&report.physical_account.baseline, "U4c glTF baseline")?;
    validate_healthy_account(&report.physical_account.retained, "U4c glTF retained")?;
    if report.physical_account.retained.live_retained_bytes != expected.retained_bytes
        || report.physical_account.retained.live_retained_allocations
            != expected.retained_allocations
        || report.physical_account.after_inspection != report.physical_account.retained
        || report.physical_account.after_snapshot != report.physical_account.retained
    {
        return Err("U4c production glTF retained physical charge drifted".to_owned());
    }
    validate_released_account(
        &report.physical_account.after_release,
        "U4c glTF final release",
    )?;
    validate_released_factory(
        &report.factory_after_account_release,
        "U4c glTF factory release",
    )
}

#[cfg(feature = "repository-input-candidate")]
fn validate_fixed_vector_report(
    report: &FixedVector,
    expected: &FixedVector,
) -> Result<(), String> {
    validate_capability_fixed_vector_report(report, expected, "U4c production glTF")
}

#[cfg(feature = "repository-input-candidate")]
fn validate_construction_phase(
    report: &PhaseReport,
    expected: &GltfCapabilitySpecification,
) -> Result<(), String> {
    let taxonomy = &report.taxonomy;
    let allocations = &report.allocations;
    let budget = &expected.construction_budget;
    if report.scenario_id != "canonical-gltf-five-domain-construction-v1"
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
        return Err("U4c production glTF construction transcript or budget drifted".to_owned());
    }
    validate_producer_bound(report, "U4c production glTF construction")
}

#[cfg(feature = "repository-input-candidate")]
fn validate_snapshot_phase(
    report: &PhaseReport,
    expected: &GltfCapabilitySpecification,
) -> Result<(), String> {
    let taxonomy = &report.taxonomy;
    let allocations = &report.allocations;
    let budget = &expected.snapshot_budget;
    validate_nonproducing_phase(report, "U4c production glTF snapshot")?;
    if report.scenario_id != "canonical-gltf-five-domain-snapshot-v1"
        || taxonomy.completed_lookups != budget.completed_lookups
        || taxonomy.rejected_lookups != 0
        || taxonomy.lookup_events != budget.lookup_events
        || taxonomy.max_lookup_events != budget.max_lookup_events
        || taxonomy.member_probes != budget.member_probes
        || taxonomy.max_member_probes_per_lookup != budget.max_member_probes_per_lookup
        || !budget.allocations.contains(allocations)
    {
        return Err("U4c production glTF snapshot transcript or budget drifted".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn managed_specification() -> GltfCapabilitySpecification {
        let specification = super::super::ProductionSpecification::parse(include_bytes!(
            "../../../../../docs/conformance/u4c-metrics.toml"
        ))
        .expect("managed U4c production specification must parse");
        specification.gltf_capability
    }

    fn assert_rejected(specification: &GltfCapabilitySpecification) {
        assert!(
            validate_specification(specification).is_err(),
            "synthetic glTF capability drift must fail closed"
        );
    }

    #[test]
    fn rejects_source_and_canonical_shape_drift() {
        let mut source_drift = managed_specification();
        source_drift.source_shape.node_count += 1;
        assert_rejected(&source_drift);

        let mut canonical_drift = managed_specification();
        canonical_drift.canonical_shape.material_count += 1;
        assert_rejected(&canonical_drift);
    }

    #[test]
    fn rejects_snapshot_and_preview_drift() {
        let mut totals_drift = managed_specification();
        totals_drift.snapshot_totals.canonical_scalar_count -= 1;
        assert_rejected(&totals_drift);

        let mut preview_drift = managed_specification();
        preview_drift.bounded_preview.bytes = 97;
        assert_rejected(&preview_drift);
    }

    #[test]
    fn rejects_construction_and_snapshot_budget_drift() {
        let mut construction_drift = managed_specification();
        construction_drift
            .construction_budget
            .max_payload_copy_bytes = 0;
        assert_rejected(&construction_drift);

        let mut snapshot_drift = managed_specification();
        snapshot_drift.snapshot_budget.max_lookup_events =
            snapshot_drift.snapshot_budget.lookup_events + 1;
        assert_rejected(&snapshot_drift);
    }

    #[test]
    fn rejects_duplicate_mutation_evidence() {
        let mut specification = managed_specification();
        specification.mutations[1].source_vector.sha256 =
            specification.mutations[0].source_vector.sha256.clone();

        assert_rejected(&specification);
    }
}
