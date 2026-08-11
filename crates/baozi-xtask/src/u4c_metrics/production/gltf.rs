use std::collections::BTreeSet;

use serde::Deserialize;

use super::capability::{
    BoundedPreview as GltfBoundedPreview, ConstructionBudgetSpecification, FixedVector,
    SnapshotBudgetSpecification,
};
#[cfg(feature = "repository-input-candidate")]
use super::capability::{
    PhysicalLifecycleReport as GltfPhysicalReport, validate_allocation_lifecycle,
    validate_bounded_preview_report,
    validate_fixed_vector_report as validate_capability_fixed_vector_report,
    validate_nonproducing_phase,
};
use super::{
    CanonicalSnapshotTotals, CoverageMappingSpecification, LedgerContractSpecification,
    is_lowercase_sha256, validate_success_dimensions,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GltfCapabilitySpecification {
    entry_point: String,
    scenario_id: String,
    source_shape: GltfSourceShape,
    canonical_shape: GltfCanonicalShape,
    source_vector: FixedVector,
    mutations: Vec<GltfMutation>,
    source_generation_reservations: u64,
    canonical_block_count: u64,
    outcome_evidence_block_count: u64,
    full_snapshot_digest_hex: String,
    snapshot_totals: CanonicalSnapshotTotals,
    bounded_preview: GltfBoundedPreview,
    construction_budget: ConstructionBudgetSpecification,
    construction_scenario_id: String,
    inspection_scenario_id: String,
    snapshot_budget: SnapshotBudgetSpecification,
    snapshot_entry_point: String,
    snapshot_scenario_id: String,
    unchanged_view_entry_point: String,
    unchanged_view_scenario_id: String,
    facade_equivalent: FacadeEquivalentSpecification,
    changed_scene_edit: ChangedSceneEditSpecification,
    release_scenario_id: String,
    construction_semantic_cells: u64,
    construction_references: u64,
    construction_blocks: u64,
    construction_producer_events: u64,
    retained_bytes: u64,
    retained_allocations: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FacadeEquivalentSpecification {
    entry_point: String,
    scenario_id: String,
    observation_schema_id: String,
    success_outcome: String,
    transcript_bytes: u64,
    transcript_digest_hex: String,
    typed_error_outcome: String,
    typed_error_scenario_id: String,
    typed_error_class: String,
    typed_error_reason: String,
    max_typed_error_reason_bytes: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangedSceneEditSpecification {
    entry_point: String,
    scenario_id: String,
    output_snapshot_digest_hex: String,
    canonical_block_count_delta: u64,
    semantic_cells: u64,
    references: u64,
    blocks: u64,
    new_blocks: u64,
    inherited_blocks: u64,
    producer_events: u64,
    completed_lookups: u64,
    lookup_events: u64,
    member_probes: u64,
    max_lookup_events: u64,
    max_member_probes_per_lookup: u64,
    budget: ConstructionBudgetSpecification,
    snapshot_budget: SnapshotBudgetSpecification,
    snapshot_scenario_id: String,
    input_release_scenario_id: String,
    output_report_release_scenario_id: String,
    output_scene_release_scenario_id: String,
    ledger_dimensions: Vec<super::LedgerDimensionReport>,
    physical_account: ChangedSceneEditPhysicalSpecification,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangedSceneEditPhysicalSpecification {
    with_output_live_bytes: u64,
    with_output_live_allocations: u64,
    after_input_release_live_bytes: u64,
    after_input_release_live_allocations: u64,
    after_output_report_release_live_bytes: u64,
    after_output_report_release_live_allocations: u64,
}

impl GltfCapabilitySpecification {
    pub(super) fn full_snapshot_digest_hex(&self) -> &str {
        &self.full_snapshot_digest_hex
    }

    pub(super) fn matches_mapping(&self, mapping: &CoverageMappingSpecification) -> bool {
        mapping.entry_point == self.entry_point && mapping.scenario_id == self.scenario_id
    }

    pub(super) fn matches_facade_mapping(&self, mapping: &CoverageMappingSpecification) -> bool {
        mapping.entry_point == self.facade_equivalent.entry_point
            && mapping.scenario_id == self.facade_equivalent.scenario_id
    }

    pub(super) fn matches_snapshot_mapping(&self, mapping: &CoverageMappingSpecification) -> bool {
        mapping.entry_point == self.snapshot_entry_point
            && mapping.scenario_id == self.snapshot_scenario_id
    }

    pub(super) fn matches_unchanged_view_mapping(
        &self,
        mapping: &CoverageMappingSpecification,
    ) -> bool {
        mapping.entry_point == self.unchanged_view_entry_point
            && mapping.scenario_id == self.unchanged_view_scenario_id
    }

    pub(super) fn matches_changed_scene_edit_mapping(
        &self,
        mapping: &CoverageMappingSpecification,
    ) -> bool {
        mapping.entry_point == self.changed_scene_edit.entry_point
            && mapping.scenario_id == self.changed_scene_edit.scenario_id
    }
}

#[cfg(feature = "repository-input-candidate")]
impl GltfCapabilityReport {
    pub(super) fn full_snapshot_digest_hex(&self) -> &str {
        &self.full_snapshot_digest_hex
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

pub(super) fn validate_specification(
    specification: &GltfCapabilitySpecification,
    ledger_contract: &LedgerContractSpecification,
) -> Result<(), String> {
    let source = &specification.source_shape;
    let canonical = &specification.canonical_shape;
    let totals = &specification.snapshot_totals;
    let preview = &specification.bounded_preview;
    let construction_budget = &specification.construction_budget;
    let snapshot_budget = &specification.snapshot_budget;
    let facade = &specification.facade_equivalent;
    let changed_edit = &specification.changed_scene_edit;
    let changed_edit_dimensions = validate_success_dimensions(
        ledger_contract,
        &changed_edit.ledger_dimensions,
        "U4c production glTF changed SceneEdit specification",
    )?;
    let Some(changed_edit_maximum_lookup_events) = changed_edit
        .completed_lookups
        .checked_mul(changed_edit.max_lookup_events)
    else {
        return Err("U4d changed SceneEdit lookup envelope overflowed".to_owned());
    };
    let Some(changed_edit_canonical_block_count_after) = specification
        .canonical_block_count
        .checked_add(changed_edit.canonical_block_count_delta)
    else {
        return Err("U4d changed SceneEdit canonical block count overflowed".to_owned());
    };
    let Some(changed_edit_published_outcome_evidence) =
        specification.outcome_evidence_block_count.checked_add(1)
    else {
        return Err("U4d changed SceneEdit outcome evidence count overflowed".to_owned());
    };
    let Some(changed_edit_published_block_count) = changed_edit_canonical_block_count_after
        .checked_add(specification.outcome_evidence_block_count)
    else {
        return Err("U4d changed SceneEdit published block count overflowed".to_owned());
    };
    let facade_reason_bytes = u64::try_from(facade.typed_error_reason.len())
        .map_err(|_| "U4d facade typed-error reason length overflowed".to_owned())?;
    if !is_bounded_identity(&specification.inspection_scenario_id)
        || !is_bounded_identity(&specification.snapshot_scenario_id)
        || !is_bounded_identity(&specification.unchanged_view_scenario_id)
        || !is_bounded_identity(&specification.release_scenario_id)
        || !is_bounded_identity(&changed_edit.snapshot_scenario_id)
        || !is_bounded_identity(&changed_edit.input_release_scenario_id)
        || !is_bounded_identity(&changed_edit.output_report_release_scenario_id)
        || !is_bounded_identity(&changed_edit.output_scene_release_scenario_id)
        || !is_bounded_identity(&facade.entry_point)
        || !is_bounded_identity(&facade.scenario_id)
        || !is_bounded_identity(&facade.observation_schema_id)
        || !is_bounded_identity(&facade.success_outcome)
        || facade.transcript_bytes == 0
        || !is_lowercase_sha256(&facade.transcript_digest_hex)
        || !is_bounded_identity(&facade.typed_error_outcome)
        || !is_bounded_identity(&facade.typed_error_scenario_id)
        || !is_bounded_identity(&facade.typed_error_class)
        || facade.typed_error_reason.is_empty()
        || facade.typed_error_reason.chars().any(char::is_control)
        || facade.max_typed_error_reason_bytes == 0
        || facade.max_typed_error_reason_bytes > 128
        || facade_reason_bytes > facade.max_typed_error_reason_bytes
        || facade.scenario_id == facade.typed_error_scenario_id
        || facade.success_outcome == facade.typed_error_outcome
    {
        return Err(
            "U4d facade-equivalent specification must declare bounded success and typed-error identities"
                .to_owned(),
        );
    }
    validate_unique_identities(specification)?;
    if !source_shape_is_well_formed(source)
        || !canonical_shape_matches_source(source, canonical)
        || specification.source_generation_reservations == 0
        || specification.canonical_block_count == 0
        || specification.outcome_evidence_block_count == 0
        || !is_lowercase_sha256(&specification.full_snapshot_digest_hex)
        || !snapshot_totals_match_shape(totals, source, canonical)
        || !preview.has_common_invariants()
        || !construction_budget.is_well_formed()
        || !snapshot_budget.is_well_formed()
        || !is_lowercase_sha256(&changed_edit.output_snapshot_digest_hex)
        || changed_edit.output_snapshot_digest_hex == specification.full_snapshot_digest_hex
        || changed_edit.canonical_block_count_delta == 0
        || changed_edit.semantic_cells == 0
        || changed_edit.references == 0
        || changed_edit.blocks == 0
        || changed_edit.new_blocks == 0
        || changed_edit.inherited_blocks == 0
        || changed_edit
            .new_blocks
            .checked_add(changed_edit.inherited_blocks)
            != Some(changed_edit.blocks)
        || changed_edit.blocks != changed_edit_published_block_count
        || changed_edit.producer_events == 0
        || changed_edit.completed_lookups == 0
        || changed_edit.lookup_events != changed_edit_maximum_lookup_events
        || changed_edit.member_probes != changed_edit.completed_lookups
        || changed_edit.max_lookup_events == 0
        || changed_edit.max_member_probes_per_lookup == 0
        || !changed_edit.budget.is_well_formed()
        || !changed_edit.snapshot_budget.is_well_formed()
        || changed_edit_dimensions.ordinary.committed != changed_edit_canonical_block_count_after
        || changed_edit_dimensions.ordinary.committed_canonical_output
            != changed_edit_canonical_block_count_after
        || changed_edit_dimensions.ordinary.published_outcome_evidence
            != changed_edit_published_outcome_evidence
        || changed_edit.physical_account.with_output_live_bytes <= specification.retained_bytes
        || changed_edit.physical_account.with_output_live_allocations
            <= specification.retained_allocations
        || changed_edit.physical_account.after_input_release_live_bytes
            >= changed_edit.physical_account.with_output_live_bytes
        || changed_edit
            .physical_account
            .after_input_release_live_allocations
            >= changed_edit.physical_account.with_output_live_allocations
        || changed_edit
            .physical_account
            .after_output_report_release_live_bytes
            >= changed_edit.physical_account.after_input_release_live_bytes
        || changed_edit
            .physical_account
            .after_output_report_release_live_allocations
            >= changed_edit
                .physical_account
                .after_input_release_live_allocations
        || changed_edit
            .physical_account
            .after_output_report_release_live_bytes
            == 0
        || changed_edit
            .physical_account
            .after_output_report_release_live_allocations
            == 0
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
            "U4c production glTF capability specification violates a structural or cross-field invariant"
                .to_owned(),
        );
    }
    validate_fixed_vector(&specification.source_vector)?;
    if specification.mutations.is_empty() {
        return Err("U4c production glTF must freeze at least one mutation vector".to_owned());
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
            || !mutation_fields.insert(mutation.field_class.as_str())
            || !mutation_vectors.insert(mutation.source_vector.sha256.as_str())
            || !mutation_snapshots.insert(mutation.snapshot_digest_hex.as_str())
        {
            return Err(format!(
                "U4c production glTF mutation {} is not independently frozen",
                mutation.field_class
            ));
        }
    }
    Ok(())
}

fn is_bounded_identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

fn validate_unique_identities(specification: &GltfCapabilitySpecification) -> Result<(), String> {
    let facade = &specification.facade_equivalent;
    let edit = &specification.changed_scene_edit;
    let entry_points = [
        specification.entry_point.as_str(),
        facade.entry_point.as_str(),
        specification.snapshot_entry_point.as_str(),
        specification.unchanged_view_entry_point.as_str(),
        edit.entry_point.as_str(),
    ];
    let scenario_ids = [
        specification.scenario_id.as_str(),
        specification.construction_scenario_id.as_str(),
        specification.inspection_scenario_id.as_str(),
        specification.snapshot_scenario_id.as_str(),
        specification.unchanged_view_scenario_id.as_str(),
        specification.release_scenario_id.as_str(),
        facade.scenario_id.as_str(),
        facade.typed_error_scenario_id.as_str(),
        edit.scenario_id.as_str(),
        edit.snapshot_scenario_id.as_str(),
        edit.input_release_scenario_id.as_str(),
        edit.output_report_release_scenario_id.as_str(),
        edit.output_scene_release_scenario_id.as_str(),
    ];
    if entry_points.iter().any(|value| !is_bounded_identity(value))
        || scenario_ids.iter().any(|value| !is_bounded_identity(value))
        || entry_points.iter().copied().collect::<BTreeSet<_>>().len() != entry_points.len()
        || scenario_ids.iter().copied().collect::<BTreeSet<_>>().len() != scenario_ids.len()
    {
        return Err(
            "U4c production glTF entry-point and scenario identities must be bounded and unique"
                .to_owned(),
        );
    }
    Ok(())
}

fn source_shape_is_well_formed(source: &GltfSourceShape) -> bool {
    source.node_count > 0
        && source.root_count > 0
        && source.root_count <= source.node_count
        && source.image_count > 0
        && source.image_payload_bytes > 0
        && source.media_type_bytes > 0
        && source.sampler_count > 0
        && source.texture_count > 0
        && source.material_count > 0
        && source.material_binding_count > 0
        && source.logical_mesh_count > 0
        && source.primitive_count > 0
        && source.attribute_count > 0
        && source.mesh_instance_count > 0
        && source.skin_count > 0
        && source.skin_joint_count > 0
        && source.morph_definition_count > 0
        && source.morph_target_count > 0
        && source.morph_delta_count > 0
        && source.influence_row_count > 0
        && source.influence_count > 0
        && source.animation_clip_count > 0
        && source.animation_channel_count > 0
        && source.animation_key_count > 0
}

fn canonical_shape_matches_source(
    source: &GltfSourceShape,
    canonical: &GltfCanonicalShape,
) -> bool {
    canonical.node_count == source.node_count
        && canonical.root_count == source.root_count
        && source.node_count.checked_sub(source.root_count) == Some(canonical.edge_count)
        && canonical.image_count == source.image_count
        && canonical.sampler_count == source.sampler_count
        && canonical.texture_count == source.texture_count
        && canonical.material_count == source.material_count
        && canonical.material_binding_count == source.material_binding_count
        && canonical.logical_mesh_count == source.logical_mesh_count
        && canonical.primitive_count == source.primitive_count
        && canonical.attribute_count == source.attribute_count
        && canonical.mesh_instance_count == source.mesh_instance_count
        && canonical.skin_count == source.skin_count
        && canonical.skin_joint_count == source.skin_joint_count
        && canonical.logical_morph_count > 0
        && canonical.logical_morph_count <= source.morph_definition_count
        && canonical.morph_definition_count == source.morph_definition_count
        && canonical.primitive_deformation_count == source.primitive_count
        && canonical.morph_target_count == source.morph_target_count
        && canonical.morph_delta_count == source.morph_delta_count
        && canonical.influence_row_count == source.influence_row_count
        && canonical.influence_count == source.influence_count
        && canonical.animation_clip_count == source.animation_clip_count
        && canonical.animation_channel_count == source.animation_channel_count
        && canonical.animation_key_count == source.animation_key_count
}

fn snapshot_totals_match_shape(
    totals: &CanonicalSnapshotTotals,
    source: &GltfSourceShape,
    canonical: &GltfCanonicalShape,
) -> bool {
    totals.node_count == canonical.node_count
        && totals.root_count == canonical.root_count
        && totals.edge_count == canonical.edge_count
        && totals.has_empty_spatial_domain()
        && totals.image_count == canonical.image_count
        && totals.sampler_count == canonical.sampler_count
        && totals.texture_count == canonical.texture_count
        && totals.material_count == canonical.material_count
        && totals.material_binding_count == canonical.material_binding_count
        && totals.image_payload_bytes == source.image_payload_bytes
        && totals.media_type_bytes == source.media_type_bytes
        && totals.binding_semantic_scalar_count > 0
        && totals.material_factor_scalar_count > 0
        && totals.logical_mesh_count == canonical.logical_mesh_count
        && totals.primitive_count == canonical.primitive_count
        && totals.attribute_count == canonical.attribute_count
        && totals.mesh_instance_count == canonical.mesh_instance_count
        && totals.primitive_index_count > 0
        && totals.canonical_scalar_count > 0
        && totals.f32_scalar_count.checked_add(totals.u32_scalar_count)
            == Some(totals.canonical_scalar_count)
        && totals.skin_count == canonical.skin_count
        && totals.logical_morph_count == canonical.logical_morph_count
        && totals.morph_definition_count == canonical.morph_definition_count
        && totals.primitive_deformation_count == canonical.primitive_deformation_count
        && totals.morph_target_count == canonical.morph_target_count
        && totals.morph_delta_count == canonical.morph_delta_count
        && totals.deformation_mesh_instance_count == canonical.mesh_instance_count
        && totals.skin_joint_count == canonical.skin_joint_count
        && totals.inverse_bind_matrix_count == canonical.skin_joint_count
        && totals.skin_name_bytes > 0
        && totals.morph_default_weight_count > 0
        && totals.morph_name_bytes > 0
        && totals.influence_row_count == canonical.influence_row_count
        && totals.influence_count == canonical.influence_count
        && totals.morph_value_vector_count > 0
        && totals.morph_sparse_index_count > 0
        && totals.instance_morph_weight_count > 0
        && totals.animation_clip_count == canonical.animation_clip_count
        && totals.animation_channel_count == canonical.animation_channel_count
        && totals.animation_time_scalar_count == canonical.animation_key_count
        && totals.animation_value_scalar_count > 0
        && totals.animation_in_tangent_scalar_count > 0
        && totals.animation_out_tangent_scalar_count > 0
        && totals.profile_resource_count > 0
        && totals.profile_source_count > 0
        && totals.profile_provenance_node_count > 0
        && totals.profile_owner_count > 0
        && totals.profile_attachment_count == totals.profile_owner_count
        && totals.profile_wide_value_count > 0
        && totals.transcript_bytes > 0
        && totals.work_units > 0
}

fn validate_fixed_vector(vector: &FixedVector) -> Result<(), String> {
    if !is_bounded_identity(&vector.schema_id)
        || vector.observed_bytes == 0
        || !is_lowercase_sha256(&vector.sha256)
    {
        return Err("U4c production glTF fixed vector is malformed".to_owned());
    }
    Ok(())
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
    source_generation_reservations: u64,
    canonical_block_count: u64,
    outcome_evidence_block_count: u64,
    full_snapshot_digest_hex: String,
    bounded_snapshot_digest_hex: String,
    snapshot_totals: CanonicalSnapshotTotals,
    bounded_preview: GltfBoundedPreview,
    evidence: super::EvidenceReport,
    construction: super::PhaseReport,
    inspection: super::PhaseReport,
    facade_equivalent: FacadeEquivalentInspectionReport,
    snapshot: super::PhaseReport,
    unchanged_view: GltfUnchangedViewReport,
    changed_scene_edit: GltfChangedSceneEditReport,
    release: super::ReleaseReport,
    physical_account: GltfPhysicalReport,
    factory_after_account_release: super::PhysicalFactoryReport,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FacadeEquivalentInspectionReport {
    entry_point: String,
    scenario_id: String,
    success: FacadeObservationSuccessReport,
    typed_error: FacadeObservationTypedErrorReport,
    success_generation_reservations_before: u64,
    success_generation_reservations_after: u64,
    success_canonical_block_count_before: u64,
    success_canonical_block_count_after: u64,
    typed_error_generation_reservations_before: u64,
    typed_error_generation_reservations_after: u64,
    typed_error_canonical_block_count_before: u64,
    typed_error_canonical_block_count_after: u64,
    success_phase: super::PhaseReport,
    typed_error_phase: super::PhaseReport,
    success_physical_account: GltfReadOnlyPhysicalReport,
    typed_error_physical_account: GltfReadOnlyPhysicalReport,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FacadeObservationSuccessReport {
    outcome: String,
    schema_id: String,
    transcript_bytes: u64,
    transcript_digest_hex: String,
    totals: GltfCanonicalShape,
    reference_closure_verified: bool,
    cursor_exhausted: bool,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FacadeObservationTypedErrorReport {
    outcome: String,
    schema_id: String,
    error_class: String,
    reason: String,
    reason_bytes: u64,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GltfUnchangedViewReport {
    observation: GltfCanonicalShape,
    repeated_observation_matches: bool,
    generation_preserved: bool,
    generation_reservations_before: u64,
    generation_reservations_after: u64,
    canonical_block_count_before: u64,
    canonical_block_count_after: u64,
    phase: super::PhaseReport,
    physical_account: GltfReadOnlyPhysicalReport,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GltfReadOnlyPhysicalReport {
    before: super::PhysicalAccountReport,
    after: super::PhysicalAccountReport,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GltfChangedSceneEditReport {
    entry_point: String,
    scenario_id: String,
    output_shape: GltfCanonicalShape,
    replacement_authored_bits_preserved: bool,
    input_authored_bits_preserved: bool,
    input_readable_after_publication: bool,
    output_report_readable_after_input_release: bool,
    output_readable_after_input_release: bool,
    output_readable_after_report_release: bool,
    exact_physical_account_preserved: bool,
    handoff_physical_account_preserved: bool,
    generation_changed: bool,
    generation_reservations_before: u64,
    generation_reservations_after: u64,
    canonical_block_count_before: u64,
    canonical_block_count_after: u64,
    all_subjects_explicitly_unchanged: bool,
    target_rebind_matches: bool,
    remap_storage_bytes: u64,
    scene_bounds_invalidated: bool,
    outcome_evidence_block_count: u64,
    evidence: super::EvidenceReport,
    phase: super::PhaseReport,
    input_snapshot_digest_hex: String,
    output_snapshot_digest_hex: String,
    snapshot: super::PhaseReport,
    input_release: super::ReleaseReport,
    output_report_release: super::ReleaseReport,
    output_scene_release: super::ReleaseReport,
    physical_account: GltfChangedSceneEditPhysicalReport,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GltfChangedSceneEditPhysicalReport {
    before: super::PhysicalAccountReport,
    before_handoff: super::PhysicalAccountReport,
    with_output: super::PhysicalAccountReport,
    after_input_release: super::PhysicalAccountReport,
    after_output_report_release: super::PhysicalAccountReport,
    after_output_scene_release: super::PhysicalAccountReport,
}

#[cfg(feature = "repository-input-candidate")]
use super::{
    PhaseReport, ProductionSpecification, classify_ledger_dimensions, validate_healthy_account,
    validate_hot_view_phase, validate_producer_bound, validate_release, validate_released_account,
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
        || report.source_generation_reservations != expected.source_generation_reservations
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
    if report.evidence.retained_evidence_block_count != report.outcome_evidence_block_count {
        return Err(
            "U4c production glTF outcome evidence count disagrees with the ledger".to_owned(),
        );
    }
    validate_success_evidence(
        &specification.ledger_contract,
        &report.evidence,
        expected.canonical_block_count,
        "U4c production glTF capability",
    )?;
    validate_construction_phase(&report.construction, expected)?;
    if report.inspection.scenario_id != expected.inspection_scenario_id {
        return Err("U4c production glTF inspection scenario identity drifted".to_owned());
    }
    validate_hot_view_phase(
        &report.inspection,
        &specification.measurement,
        "U4c production glTF typed inspection",
    )?;
    validate_nonproducing_phase(&report.inspection, "U4c production glTF typed inspection")?;
    validate_snapshot_phase(&report.snapshot, expected)?;
    validate_facade_equivalent(report, expected, &specification.measurement)?;
    validate_unchanged_view(report, expected, &specification.measurement)?;
    validate_changed_scene_edit(report, specification)?;
    if report.release.scenario_id != expected.release_scenario_id {
        return Err("U4c production glTF release scenario identity drifted".to_owned());
    }
    validate_release(&report.release, "U4c production glTF release")?;
    validate_allocation_lifecycle(
        [
            &report.construction.allocations,
            &report.inspection.allocations,
            &report.snapshot.allocations,
            &report.facade_equivalent.success_phase.allocations,
            &report.unchanged_view.phase.allocations,
            &report.changed_scene_edit.phase.allocations,
            &report.changed_scene_edit.snapshot.allocations,
            &report.facade_equivalent.typed_error_phase.allocations,
            &report.changed_scene_edit.input_release.allocations,
            &report.changed_scene_edit.output_report_release.allocations,
            &report.changed_scene_edit.output_scene_release.allocations,
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
fn validate_facade_equivalent(
    report: &GltfCapabilityReport,
    expected: &GltfCapabilitySpecification,
    measurement: &super::MeasurementSpecification,
) -> Result<(), String> {
    let facade = &report.facade_equivalent;
    let expected_facade = &expected.facade_equivalent;
    let typed_error_generation = expected
        .source_generation_reservations
        .checked_add(1)
        .ok_or_else(|| "U4d facade typed-error generation count overflowed".to_owned())?;
    let reason_bytes = u64::try_from(facade.typed_error.reason.len())
        .map_err(|_| "U4d facade typed-error reason length overflowed".to_owned())?;
    if facade.entry_point != expected_facade.entry_point
        || facade.scenario_id != expected_facade.scenario_id
        || facade.success.outcome != expected_facade.success_outcome
        || facade.success.schema_id != expected_facade.observation_schema_id
        || facade.success.transcript_bytes != expected_facade.transcript_bytes
        || facade.success.transcript_digest_hex != expected_facade.transcript_digest_hex
        || facade.success.totals != expected.canonical_shape
        || !facade.success.reference_closure_verified
        || !facade.success.cursor_exhausted
        || facade.typed_error.outcome != expected_facade.typed_error_outcome
        || facade.typed_error.schema_id != expected_facade.observation_schema_id
        || facade.typed_error.error_class != expected_facade.typed_error_class
        || facade.typed_error.reason != expected_facade.typed_error_reason
        || facade.typed_error.reason_bytes != reason_bytes
        || facade.typed_error.reason_bytes > expected_facade.max_typed_error_reason_bytes
        || facade.success_generation_reservations_before != expected.source_generation_reservations
        || facade.success_generation_reservations_after
            != facade.success_generation_reservations_before
        || facade.success_canonical_block_count_before != expected.canonical_block_count
        || facade.success_canonical_block_count_after != facade.success_canonical_block_count_before
        || facade.typed_error_generation_reservations_before != typed_error_generation
        || facade.typed_error_generation_reservations_after
            != facade.typed_error_generation_reservations_before
        || facade.typed_error_canonical_block_count_before != expected.canonical_block_count
        || facade.typed_error_canonical_block_count_after
            != facade.typed_error_canonical_block_count_before
        || facade.success_phase.scenario_id != expected_facade.scenario_id
        || facade.typed_error_phase.scenario_id != expected_facade.typed_error_scenario_id
        || facade.success_physical_account.before != report.physical_account.after_snapshot
        || facade.success_physical_account.after != facade.success_physical_account.before
        || facade.typed_error_physical_account.before
            != report.changed_scene_edit.physical_account.with_output
        || facade.typed_error_physical_account.after != facade.typed_error_physical_account.before
    {
        return Err("U4d facade-equivalent observation or typed-error receipt drifted".to_owned());
    }
    validate_hot_view_phase(
        &facade.success_phase,
        measurement,
        "U4d facade-equivalent success",
    )?;
    validate_nonproducing_phase(&facade.success_phase, "U4d facade-equivalent success")?;
    validate_hot_view_phase(
        &facade.typed_error_phase,
        measurement,
        "U4d facade-equivalent typed error",
    )?;
    validate_nonproducing_phase(
        &facade.typed_error_phase,
        "U4d facade-equivalent typed error",
    )
}

#[cfg(feature = "repository-input-candidate")]
fn validate_unchanged_view(
    report: &GltfCapabilityReport,
    expected: &GltfCapabilitySpecification,
    measurement: &super::MeasurementSpecification,
) -> Result<(), String> {
    let unchanged = &report.unchanged_view;
    if unchanged.observation != report.canonical_shape
        || !unchanged.repeated_observation_matches
        || !unchanged.generation_preserved
        || unchanged.generation_reservations_before != expected.source_generation_reservations
        || unchanged.generation_reservations_after != expected.source_generation_reservations
        || unchanged.canonical_block_count_before != expected.canonical_block_count
        || unchanged.canonical_block_count_after != expected.canonical_block_count
        || unchanged.physical_account.before != report.physical_account.retained
        || unchanged.physical_account.after != unchanged.physical_account.before
    {
        return Err("U4c production glTF unchanged-view invariants drifted".to_owned());
    }
    if unchanged.phase.scenario_id != expected.unchanged_view_scenario_id {
        return Err("U4c production glTF unchanged-view identity drifted".to_owned());
    }
    validate_hot_view_phase(
        &unchanged.phase,
        measurement,
        "U4c production glTF unchanged view",
    )?;
    validate_nonproducing_phase(&unchanged.phase, "U4c production glTF unchanged view")
}

#[cfg(feature = "repository-input-candidate")]
fn validate_changed_scene_edit(
    report: &GltfCapabilityReport,
    specification: &ProductionSpecification,
) -> Result<(), String> {
    let expected = &specification.gltf_capability;
    let edit = &report.changed_scene_edit;
    let expected_edit = &expected.changed_scene_edit;
    let actual_dimensions = classify_ledger_dimensions(
        &specification.ledger_contract,
        &edit.evidence.dimensions,
        "U4c production glTF changed SceneEdit report",
    )?;
    let expected_dimensions = classify_ledger_dimensions(
        &specification.ledger_contract,
        &expected_edit.ledger_dimensions,
        "U4c production glTF changed SceneEdit specification",
    )?;
    let exact_dimensions_match = actual_dimensions.ordinary == expected_dimensions.ordinary
        && actual_dimensions.failure_terminal == expected_dimensions.failure_terminal;
    let expected_canonical_block_count_after = expected
        .canonical_block_count
        .checked_add(expected_edit.canonical_block_count_delta)
        .ok_or_else(|| {
            "U4c production glTF changed edit canonical block count overflowed".to_owned()
        })?;
    let expected_published_block_count = edit
        .canonical_block_count_after
        .checked_add(edit.outcome_evidence_block_count)
        .ok_or_else(|| {
            "U4c production glTF changed edit published block count overflowed".to_owned()
        })?;
    if edit.entry_point != expected_edit.entry_point
        || edit.scenario_id != expected_edit.scenario_id
        || edit.output_shape != report.canonical_shape
        || !edit.replacement_authored_bits_preserved
        || !edit.input_authored_bits_preserved
        || !edit.input_readable_after_publication
        || !edit.output_report_readable_after_input_release
        || !edit.output_readable_after_input_release
        || !edit.output_readable_after_report_release
        || !edit.exact_physical_account_preserved
        || !edit.handoff_physical_account_preserved
        || !edit.generation_changed
        || edit.generation_reservations_before != expected.source_generation_reservations
        || edit.generation_reservations_after
            != edit
                .generation_reservations_before
                .checked_add(1)
                .ok_or_else(|| {
                    "U4c production glTF changed edit generation count overflowed".to_owned()
                })?
        || edit.canonical_block_count_before != expected.canonical_block_count
        || edit
            .canonical_block_count_after
            .checked_sub(edit.canonical_block_count_before)
            != Some(expected_edit.canonical_block_count_delta)
        || edit.canonical_block_count_after != expected_canonical_block_count_after
        || !edit.all_subjects_explicitly_unchanged
        || !edit.target_rebind_matches
        || edit.remap_storage_bytes != 0
        || !edit.scene_bounds_invalidated
        || edit.outcome_evidence_block_count == 0
        || edit.evidence.retained_evidence_block_count != edit.outcome_evidence_block_count
        || !exact_dimensions_match
        || !is_lowercase_sha256(&edit.input_snapshot_digest_hex)
        || !is_lowercase_sha256(&edit.output_snapshot_digest_hex)
        || edit.input_snapshot_digest_hex != report.full_snapshot_digest_hex
        || edit.output_snapshot_digest_hex != expected_edit.output_snapshot_digest_hex
        || edit.output_snapshot_digest_hex == report.full_snapshot_digest_hex
    {
        return Err("U4c production glTF changed-scene-edit semantics drifted".to_owned());
    }
    validate_success_evidence(
        &specification.ledger_contract,
        &edit.evidence,
        expected_canonical_block_count_after,
        "U4c production glTF changed SceneEdit",
    )?;

    let taxonomy = &edit.phase.taxonomy;
    if edit.phase.scenario_id != edit.scenario_id
        || taxonomy.semantic_cell_writes != expected_edit.semantic_cells
        || taxonomy.reference_writes != expected_edit.references
        || taxonomy.block_begins != expected_published_block_count
        || taxonomy.block_begins != expected_edit.blocks
        || taxonomy.block_seals != expected_edit.blocks
        || taxonomy.slot_publications != expected_edit.blocks
        || taxonomy.new_block_begins != expected_edit.new_blocks
        || taxonomy.new_block_seals != expected_edit.new_blocks
        || taxonomy.new_slot_publications != expected_edit.new_blocks
        || taxonomy.inherited_block_begins != expected_edit.inherited_blocks
        || taxonomy.inherited_block_seals != expected_edit.inherited_blocks
        || taxonomy.inherited_slot_publications != expected_edit.inherited_blocks
        || taxonomy.producer_events != expected_edit.producer_events
        || taxonomy.payload_copy_bytes > expected_edit.budget.max_payload_copy_bytes
        || taxonomy.payload_backing_allocations != taxonomy.new_block_begins
        || taxonomy.element_decodes != 0
        || taxonomy.completed_lookups != expected_edit.completed_lookups
        || taxonomy.rejected_lookups != 0
        || taxonomy.lookup_events != expected_edit.lookup_events
        || taxonomy.max_lookup_events != expected_edit.max_lookup_events
        || taxonomy.member_probes != expected_edit.member_probes
        || taxonomy.max_member_probes_per_lookup != expected_edit.max_member_probes_per_lookup
        || !expected_edit
            .budget
            .allocations
            .contains(&edit.phase.allocations)
    {
        return Err("U4c production glTF changed-scene-edit transcript drifted".to_owned());
    }
    validate_producer_bound(&edit.phase, "U4c production glTF changed SceneEdit")?;
    let snapshot_taxonomy = &edit.snapshot.taxonomy;
    let snapshot_budget = &expected_edit.snapshot_budget;
    if edit.snapshot.scenario_id != expected_edit.snapshot_scenario_id
        || snapshot_taxonomy.completed_lookups != snapshot_budget.completed_lookups
        || snapshot_taxonomy.rejected_lookups != 0
        || snapshot_taxonomy.lookup_events != snapshot_budget.lookup_events
        || snapshot_taxonomy.member_probes != snapshot_budget.member_probes
        || snapshot_taxonomy.max_lookup_events != snapshot_budget.max_lookup_events
        || snapshot_taxonomy.max_member_probes_per_lookup
            != snapshot_budget.max_member_probes_per_lookup
        || !snapshot_budget
            .allocations
            .contains(&edit.snapshot.allocations)
    {
        return Err("U4c production glTF changed-scene-edit snapshot identity drifted".to_owned());
    }
    validate_nonproducing_phase(
        &edit.snapshot,
        "U4c production glTF changed SceneEdit snapshot",
    )?;

    if edit.input_release.scenario_id != expected_edit.input_release_scenario_id
        || edit.output_report_release.scenario_id != expected_edit.output_report_release_scenario_id
        || edit.output_scene_release.scenario_id != expected_edit.output_scene_release_scenario_id
    {
        return Err("U4c production glTF changed-scene-edit release identity drifted".to_owned());
    }
    validate_release(
        &edit.input_release,
        "U4c production glTF changed SceneEdit input release",
    )?;
    validate_release(
        &edit.output_report_release,
        "U4c production glTF changed SceneEdit report release",
    )?;
    validate_release(
        &edit.output_scene_release,
        "U4c production glTF changed SceneEdit output release",
    )?;

    let physical = &edit.physical_account;
    validate_healthy_account(&physical.before, "U4c glTF changed edit input")?;
    validate_healthy_account(
        &physical.before_handoff,
        "U4c glTF changed edit prepared publication",
    )?;
    validate_healthy_account(&physical.with_output, "U4c glTF changed edit publication")?;
    validate_healthy_account(
        &physical.after_input_release,
        "U4c glTF changed edit after input release",
    )?;
    validate_healthy_account(
        &physical.after_output_report_release,
        "U4c glTF changed edit after report release",
    )?;
    validate_released_account(
        &physical.after_output_scene_release,
        "U4c glTF changed edit after output release",
    )?;
    if physical.before != report.physical_account.after_snapshot
        || physical.before != report.unchanged_view.physical_account.after
        || physical.before_handoff != physical.with_output
        || physical.with_output.live_retained_bytes <= physical.before.live_retained_bytes
        || physical.with_output.live_retained_allocations
            <= physical.before.live_retained_allocations
        || physical.after_input_release.live_retained_bytes == 0
        || physical.after_input_release.live_retained_allocations == 0
        || physical.after_input_release.live_retained_bytes
            >= physical.with_output.live_retained_bytes
        || physical.after_input_release.live_retained_allocations
            >= physical.with_output.live_retained_allocations
        || physical.after_output_report_release.live_retained_bytes == 0
        || physical
            .after_output_report_release
            .live_retained_allocations
            == 0
        || physical.after_output_report_release.live_retained_bytes
            >= physical.after_input_release.live_retained_bytes
        || physical
            .after_output_report_release
            .live_retained_allocations
            >= physical.after_input_release.live_retained_allocations
        || physical.with_output.live_retained_bytes
            != expected_edit.physical_account.with_output_live_bytes
        || physical.with_output.live_retained_allocations
            != expected_edit.physical_account.with_output_live_allocations
        || physical.after_input_release.live_retained_bytes
            != expected_edit
                .physical_account
                .after_input_release_live_bytes
        || physical.after_input_release.live_retained_allocations
            != expected_edit
                .physical_account
                .after_input_release_live_allocations
        || physical.after_output_report_release.live_retained_bytes
            != expected_edit
                .physical_account
                .after_output_report_release_live_bytes
        || physical
            .after_output_report_release
            .live_retained_allocations
            != expected_edit
                .physical_account
                .after_output_report_release_live_allocations
    {
        return Err("U4c production glTF changed-scene-edit physical ownership drifted".to_owned());
    }
    Ok(())
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
    if report.scenario_id != expected.snapshot_scenario_id
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

    fn managed_ledger_contract() -> super::super::LedgerContractSpecification {
        let specification = super::super::ProductionSpecification::parse(include_bytes!(
            "../../../../../docs/conformance/u4c-metrics.toml"
        ))
        .expect("managed U4c production specification must parse");
        specification.ledger_contract
    }

    fn assert_rejected(specification: &GltfCapabilitySpecification) {
        let ledger_contract = managed_ledger_contract();
        assert!(
            validate_specification(specification, &ledger_contract).is_err(),
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

        let mut profile_drift = managed_specification();
        profile_drift.snapshot_totals.profile_resource_count = 0;
        assert_rejected(&profile_drift);

        let mut spatial_drift = managed_specification();
        spatial_drift.snapshot_totals.camera_count += 1;
        assert_rejected(&spatial_drift);

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
    fn rejects_facade_identity_and_error_contract_drift() {
        let mut schema_drift = managed_specification();
        schema_drift.facade_equivalent.observation_schema_id.clear();
        assert_rejected(&schema_drift);

        let mut outcome_drift = managed_specification();
        outcome_drift.facade_equivalent.success_outcome =
            outcome_drift.facade_equivalent.typed_error_outcome.clone();
        assert_rejected(&outcome_drift);

        let mut scenario_drift = managed_specification();
        scenario_drift.facade_equivalent.typed_error_scenario_id =
            scenario_drift.facade_equivalent.scenario_id.clone();
        assert_rejected(&scenario_drift);

        let mut reason_bound_drift = managed_specification();
        reason_bound_drift
            .facade_equivalent
            .max_typed_error_reason_bytes = 129;
        assert_rejected(&reason_bound_drift);
    }

    #[test]
    fn rejects_changed_scene_edit_copy_and_mapping_drift() {
        let mut delta_drift = managed_specification();
        delta_drift.changed_scene_edit.canonical_block_count_delta += 1;
        assert_rejected(&delta_drift);

        let mut block_drift = managed_specification();
        block_drift.changed_scene_edit.inherited_blocks -= 1;
        assert_rejected(&block_drift);

        let mut unowned_block_drift = managed_specification();
        unowned_block_drift.changed_scene_edit.blocks += 1;
        unowned_block_drift.changed_scene_edit.new_blocks += 1;
        assert_rejected(&unowned_block_drift);

        let mut lookup_drift = managed_specification();
        lookup_drift.changed_scene_edit.lookup_events -= 1;
        assert_rejected(&lookup_drift);

        let mut budget_drift = managed_specification();
        budget_drift
            .changed_scene_edit
            .budget
            .max_payload_copy_bytes = 0;
        assert_rejected(&budget_drift);

        let mut snapshot_drift = managed_specification();
        snapshot_drift
            .changed_scene_edit
            .snapshot_budget
            .completed_lookups = 0;
        assert_rejected(&snapshot_drift);

        let mut ledger_drift = managed_specification();
        let ordinary = ledger_drift
            .changed_scene_edit
            .ledger_dimensions
            .iter_mut()
            .find(|dimension| dimension.role == super::super::LedgerDimensionRole::Ordinary)
            .expect("managed changed SceneEdit ordinary dimension");
        ordinary.semantic_id_hex = "00".to_owned();
        assert_rejected(&ledger_drift);

        let mut outcome_evidence_drift = managed_specification();
        let ordinary = outcome_evidence_drift
            .changed_scene_edit
            .ledger_dimensions
            .iter_mut()
            .find(|dimension| dimension.role == super::super::LedgerDimensionRole::Ordinary)
            .expect("managed changed SceneEdit ordinary dimension");
        ordinary.published_outcome_evidence += 1;
        assert_rejected(&outcome_evidence_drift);

        let mut protected_drift = managed_specification();
        let failure_terminal = protected_drift
            .changed_scene_edit
            .ledger_dimensions
            .iter_mut()
            .find(|dimension| dimension.role == super::super::LedgerDimensionRole::FailureTerminal)
            .expect("managed changed SceneEdit failure-terminal dimension");
        failure_terminal.attempted = 1;
        assert_rejected(&protected_drift);

        let mut physical_drift = managed_specification();
        physical_drift
            .changed_scene_edit
            .physical_account
            .with_output_live_bytes = physical_drift.retained_bytes;
        assert_rejected(&physical_drift);
    }

    #[test]
    fn rejects_duplicate_mutation_evidence() {
        let mut specification = managed_specification();
        specification.mutations[1].source_vector.sha256 =
            specification.mutations[0].source_vector.sha256.clone();

        assert_rejected(&specification);
    }
}
