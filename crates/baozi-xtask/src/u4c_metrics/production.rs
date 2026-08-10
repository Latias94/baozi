//! U4c production measurement admission.
//!
//! This module deliberately has no filesystem fallback. Production evidence must
//! enter through the repository-owned, handle-relative capture capability.

#![cfg_attr(
    not(any(feature = "repository-input-candidate", test)),
    allow(
        dead_code,
        reason = "the default build retains only the fail-closed production route"
    )
)]

mod build_output;
mod capability;
mod gltf;
mod obj;
mod profile;

use build_output::BuildOutputSpecification;
#[cfg(feature = "repository-input-candidate")]
use build_output::{
    BuildOutputAuthority, BuildPackageIdentity,
    BuildPackageReference as BuildOutputPackageReference, PreparedArtifactExpectation,
    PreparedCargoArtifact, RetainedBuildOutputCapture, parse_prepared_artifact,
};
use serde::Deserialize;
use serde_json::Value;
#[cfg(feature = "repository-input-candidate")]
use serde_json::json;
#[cfg(feature = "repository-input-candidate")]
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
#[cfg(feature = "repository-input-candidate")]
use std::collections::{BTreeMap, VecDeque};
#[cfg(feature = "repository-input-candidate")]
use std::env;
#[cfg(feature = "repository-input-candidate")]
use std::fs::{self, File};
#[cfg(feature = "repository-input-candidate")]
use std::io::{ErrorKind, Read};
#[cfg(feature = "repository-input-candidate")]
use std::path::PathBuf;
use std::path::{Component, Path};
#[cfg(feature = "repository-input-candidate")]
use std::process::{self, Command};
#[cfg(feature = "repository-input-candidate")]
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) const SPECIFICATION_PATH: &str = "docs/conformance/u4c-metrics.toml";
const SPECIFICATION_SCHEMA_VERSION: u64 = 7;
const PRODUCTION_SCOPE: &str = "u4c-production";
const PRODUCTION_PROBE_PACKAGE: &str = "baozi-core";
#[cfg(feature = "repository-input-candidate")]
const PRODUCTION_PROBE_CARGO_TARGET: &str = "baozi_core";
const PRODUCTION_PROBE_TARGET: &str =
    "canonical_scene::store::tests::production_probe::canonical_store_u4c_production_v2_probe";
const PRODUCTION_REPORT_PREFIX: &str = "BAOZI_U4C_PRODUCTION_V2=";
const PRODUCTION_REPORT_SCHEMA_VERSION: u64 = 2;
const PRODUCTION_REPORT_REVISION: u64 = 3;
const PRODUCTION_SCENARIO_SET_ID: &str = "canonical-store-production-v2";
const MINIMAL_FALLBACK_SCENARIO_ID: &str = "canonical-node-minimal-fallback-v1";
const PENDING_SCENE_ACTIVATION_ENTRY_POINT: &str = "pending-scene-activation";
const PENDING_SCENE_ACTIVATION_SCENARIO_ID: &str = "canonical-node-pending-scene-activation-v1";
const PENDING_SCENE_ACTIVATION_CANONICAL_BLOCKS: u64 = 4;
const PENDING_SCENE_ACTIVATION_OUTCOME_EVIDENCE_BLOCKS: u64 = 1;
const PENDING_SCENE_ACTIVATION_NEW_BLOCKS: u64 = 5;
const PENDING_SCENE_ACTIVATION_SEMANTIC_CELLS: u64 = 62;
const PENDING_SCENE_ACTIVATION_REFERENCES: u64 = 5;
const PENDING_SCENE_ACTIVATION_PRODUCER_EVENTS: u64 = 82;
const PROTECTED_TERMINAL_ACTIVATION_SCENARIO_ID: &str =
    "canonical-node-protected-terminal-activation-v1";
const ORDINARY_DIMENSION_SEMANTIC_ID_HEX: &str =
    "0262616f7a692d636f72652d7363656e65000000010000000000000001";
const FAILURE_TERMINAL_DIMENSION_SEMANTIC_ID_HEX: &str =
    "0262616f7a692d636f72652d7363656e65000000010000000000000002";
#[cfg(feature = "repository-input-candidate")]
const XTASK_PACKAGE: &str = "baozi-xtask";
#[cfg(feature = "repository-input-candidate")]
const XTASK_CARGO_TARGET: &str = "baozi-xtask";
const COLLECTOR_PACKAGE: &str = "baozi-operation";
const COLLECTOR_TEST_TARGET: &str = "u4c_collector";
const COLLECTOR_FEATURE: &str = "_u4c-collector";
const TRUSTED_STATIC_WORKTREE: &str = "trusted-static-worktree";
const REQUIRED_SOURCE_TREES: &[&str] = &["crates"];
const REQUIRED_FIXED_INPUTS: &[&str] = &["Cargo.lock", "Cargo.toml"];
const REQUIRED_COVERAGE_ENTRY_POINTS: &[&str] = &[
    "canonical-obj-corner-smoothing",
    "backend-free-gltf-shape",
    "canonical-profile-closure",
    "pending-scene-activation",
    "facade-equivalent-inspection",
    "bounded-snapshot",
    "unchanged-view",
    "changed-scene-edit",
];
const MEASURED_PRIVATE_TRACER_COVERAGE: &[(&str, &str)] = &[
    (
        "facade-equivalent-inspection",
        "canonical-gltf-five-domain-inspection-v1",
    ),
    ("bounded-snapshot", "canonical-gltf-five-domain-snapshot-v1"),
    (
        "unchanged-view",
        "canonical-gltf-five-domain-unchanged-view-v1",
    ),
    (
        "changed-scene-edit",
        "canonical-gltf-node-transform-edit-v1",
    ),
];
const REQUIRED_SCALE_SCENARIOS: &[&str] = &[
    "complex-members-n-v1",
    "complex-members-2n-v1",
    "complex-members-4n-v1",
];
const REQUIRED_MUTATION_SCENARIOS: &[&str] = &[
    "complex.scalar.texture-image-index.v1",
    "complex.reference.texture-images.v1",
    "complex.sequence.inverse-bind.v1",
    "complex.utf8.image-mime.v1",
    "complex.bytes.image-payload.v1",
    "complex.orphan.canonical-block.v1",
];
const SUPPORTED_REPOSITORY_TARGETS: &[&str] = &["x86_64-unknown-linux-gnu"];
#[cfg(feature = "repository-input-candidate")]
static CARGO_SANDBOX_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProductionSpecification {
    schema_version: u64,
    evidence_scope: String,
    runner: RunnerSpecification,
    report_contract: ReportContractSpecification,
    ledger_contract: LedgerContractSpecification,
    pending_scene_activation: PendingSceneActivationSpecification,
    protected_terminal_activation: ProtectedTerminalActivationSpecification,
    minimal_fallback: MinimalFallbackSpecification,
    identity: IdentitySpecification,
    closure: ClosureSpecification,
    collector_gate: CollectorGateSpecification,
    semantic_vectors: Vec<SemanticVectorSpecification>,
    mutations: Vec<MutationSpecification>,
    scenarios: Vec<ScaleScenarioSpecification>,
    obj_capability: obj::ObjCapabilitySpecification,
    gltf_capability: gltf::GltfCapabilitySpecification,
    profile_closure: profile::ProfileClosureSpecification,
    measurement: MeasurementSpecification,
    coverage: CoverageSpecification,
    #[cfg(feature = "repository-input-candidate")]
    #[serde(skip)]
    specification_sha256: String,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CanonicalSnapshotTotals {
    node_count: u64,
    root_count: u64,
    edge_count: u64,
    camera_count: u64,
    light_count: u64,
    camera_attachment_count: u64,
    light_attachment_count: u64,
    image_count: u64,
    sampler_count: u64,
    texture_count: u64,
    material_count: u64,
    material_binding_count: u64,
    image_payload_bytes: u64,
    media_type_bytes: u64,
    source_key_bytes: u64,
    binding_semantic_scalar_count: u64,
    material_factor_scalar_count: u64,
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
    profile_resource_count: u64,
    profile_source_count: u64,
    profile_provenance_node_count: u64,
    profile_operation_count: u64,
    profile_evidence_count: u64,
    profile_owner_count: u64,
    profile_attachment_count: u64,
    profile_metadata_entry_count: u64,
    profile_parent_reference_count: u64,
    profile_wide_value_count: u64,
    profile_lineage_text_bytes: u64,
    profile_lineage_digest_bytes: u64,
    profile_metadata_text_bytes: u64,
    profile_metadata_payload_bytes: u64,
    profile_metadata_bool_scalar_count: u64,
    profile_metadata_i64_scalar_count: u64,
    profile_metadata_u64_scalar_count: u64,
    profile_metadata_f64_scalar_count: u64,
    profile_metadata_f32_scalar_count: u64,
    transcript_bytes: u64,
    work_units: u64,
}

impl CanonicalSnapshotTotals {
    fn has_empty_spatial_domain(&self) -> bool {
        self.camera_count == 0
            && self.light_count == 0
            && self.camera_attachment_count == 0
            && self.light_attachment_count == 0
    }

    fn has_fresh_profile(&self, expected_owner_count: u64) -> bool {
        self.profile_resource_count == 1
            && self.profile_source_count == 1
            && self.profile_provenance_node_count == 1
            && self.profile_operation_count == 0
            && self.profile_evidence_count == 0
            && self.profile_owner_count == expected_owner_count
            && self.profile_attachment_count == expected_owner_count
            && self.profile_metadata_entry_count == 0
            && self.profile_parent_reference_count == 0
            && self.profile_wide_value_count == 1
            && self.profile_lineage_text_bytes == 42
            && self.profile_lineage_digest_bytes == 0
            && self.profile_metadata_text_bytes == 0
            && self.profile_metadata_payload_bytes == 0
            && self.profile_metadata_bool_scalar_count == 0
            && self.profile_metadata_i64_scalar_count == 0
            && self.profile_metadata_u64_scalar_count == 0
            && self.profile_metadata_f64_scalar_count == 0
            && self.profile_metadata_f32_scalar_count == 0
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunnerSpecification {
    probe_package: String,
    probe_target: String,
    jobs: u64,
    test_threads: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReportContractSpecification {
    prefix: String,
    schema_id: String,
    schema_version: u64,
    report_revision: u64,
    scenario_set_id: String,
    payload_bytes: u64,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LedgerContractSpecification {
    ordinary_semantic_id_hex: String,
    failure_terminal_semantic_id_hex: String,
    rich_failure_protected_attempted_work: u64,
    rich_failure_protected_consumed_work: u64,
    minimal_fallback_protected_attempted_work: u64,
    minimal_fallback_protected_consumed_work: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MinimalFallbackSpecification {
    scenario_id: String,
    fallback_code: u32,
    terminal_failure_class: String,
    terminal_failure_code: u32,
    fallback_output_count: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProtectedTerminalActivationSpecification {
    scenario_id: String,
    terminal_failure_class: String,
    terminal_failure_code: u32,
    retained_evidence_blocks: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingSceneActivationSpecification {
    entry_point: String,
    scenario_id: String,
    canonical_block_count: u64,
    outcome_evidence_block_count: u64,
    new_block_count: u64,
    semantic_cells: u64,
    references: u64,
    producer_events: u64,
    max_payload_copy_bytes: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentitySpecification {
    rustc_release: String,
    rustup_toolchain: String,
    host: String,
    target: String,
    profile: String,
    allocator: String,
    rustc_verbose_sha256: String,
    cargo_verbose_sha256: String,
    libstd_sha256: String,
    toolchain_file_count: u64,
    toolchain_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClosureSpecification {
    worktree_premise: String,
    limits: CaptureLimitsSpecification,
    build_outputs: BuildOutputSpecification,
    source_trees: Vec<SourceTreeSpecification>,
    fixed_inputs: Vec<FixedInputSpecification>,
    packages: Vec<PackageSpecification>,
    build_scripts: Vec<PackageReference>,
    proc_macros: Vec<PackageReference>,
    generated_outputs: Vec<GeneratedOutputSpecification>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureLimitsSpecification {
    max_roots: u64,
    max_entries: u64,
    max_files: u64,
    max_directories: u64,
    max_depth: u64,
    max_path_bytes: u64,
    max_file_bytes: u64,
    max_total_bytes: u64,
    max_work: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceTreeSpecification {
    path: String,
    file_count: u64,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixedInputSpecification {
    path: String,
    sha256: String,
}

#[derive(Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
struct PackageSpecification {
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
    repository_manifest: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
struct PackageReference {
    name: String,
    version: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GeneratedOutputSpecification {
    path: String,
    generator: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CollectorGateSpecification {
    package: String,
    test_target: String,
    feature: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SemanticVectorSpecification {
    scenario_id: String,
    observed_bytes: u64,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MutationSpecification {
    scenario_id: String,
    field_class: String,
    transcript_bytes: u64,
    transcript_sha256: String,
    failure_code: u64,
    retained_evidence_blocks: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScaleScenarioSpecification {
    scenario_id: String,
    image_count: u64,
    canonical_block_count: u64,
    semantic_cells: u64,
    references: u64,
    blocks: u64,
    producer_events: u64,
    target_live_retained_bytes: u64,
    target_live_retained_allocations: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MeasurementSpecification {
    scale_completed_lookups: u64,
    scale_rejected_lookups: u64,
    scale_lookup_events: u64,
    scale_member_probes: u64,
    max_lookup_events: u64,
    max_member_probes: u64,
    hot_view_max_allocations: u64,
    hot_view_max_element_decodes: u64,
    hot_view_max_payload_copy_bytes: u64,
    hot_view_max_payload_backing_allocations: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CoverageSpecification {
    required_entry_points: Vec<String>,
    mapped_entry_points: Vec<CoverageMappingSpecification>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CoverageMappingSpecification {
    entry_point: String,
    scenario_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProductionEvidenceSummary {
    declared_private_tracer_mapping_complete: bool,
    qualifies_u4d_exit: bool,
}

impl ProductionEvidenceSummary {
    fn after_validated_private_tracer_report() -> Self {
        Self {
            // The parsed specification and observed report already proved the complete frozen
            // private-tracer mapping. This comparator does not evaluate the full U4d exit gate.
            declared_private_tracer_mapping_complete: true,
            qualifies_u4d_exit: false,
        }
    }
}

pub(super) fn collect(repository_root: &Path, specification_path: &Path) -> Result<Value, String> {
    if !is_canonical_specification_path(repository_root, specification_path) {
        return Err(format!(
            "U4c production evidence requires the canonical specification {SPECIFICATION_PATH:?}"
        ));
    }

    collect_canonical_specification(repository_root)
}

pub(super) fn is_canonical_specification_path(
    repository_root: &Path,
    specification_path: &Path,
) -> bool {
    let relative = if specification_path.is_absolute() {
        match specification_path.strip_prefix(repository_root) {
            Ok(path) => path,
            Err(_) => return false,
        }
    } else {
        specification_path
    };

    relative
        .components()
        .eq(Path::new(SPECIFICATION_PATH).components())
        && relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

#[cfg(not(feature = "repository-input-candidate"))]
fn collect_canonical_specification(_repository_root: &Path) -> Result<Value, String> {
    Err(
        "U4c production evidence requires baozi-xtask feature \"repository-input-candidate\"; \
         no canonicalize-and-read fallback is available"
            .to_owned(),
    )
}

#[cfg(feature = "repository-input-candidate")]
fn collect_canonical_specification(repository_root: &Path) -> Result<Value, String> {
    let repository = baozi_io::RepositoryRoot::open_ambient(repository_root)
        .map_err(|error| format!("failed to open the U4c repository capture root: {error}"))?;
    let bootstrap_limits = capture_limits(&CaptureLimitsSpecification {
        max_roots: 1,
        max_entries: 8,
        max_files: 4,
        max_directories: 4,
        max_depth: 8,
        max_path_bytes: 512,
        max_file_bytes: 1_048_576,
        max_total_bytes: 2_097_152,
        max_work: 4_096,
    })?;
    let bootstrap = repository
        .capture(
            vec![repository_file_root(SPECIFICATION_PATH)?],
            bootstrap_limits,
        )
        .map_err(|error| format!("failed to capture the U4c production specification: {error}"))?;
    let specification_bytes = captured_file_bytes(&bootstrap, SPECIFICATION_PATH)?;
    let mut specification = ProductionSpecification::parse(specification_bytes)?;
    specification.specification_sha256 = sha256_hex(specification_bytes);

    let closure = capture_declared_closure(&repository, &specification)?;
    verify_bootstrap_specification_bytes(
        specification_bytes,
        captured_file_bytes(&closure, SPECIFICATION_PATH)?,
    )?;
    let closure_summary = verify_captured_closure(&closure, &specification)?;
    let toolchain = VerifiedToolchain::admit(&specification.identity)?;
    let cargo_sandbox = CargoSandbox::create(repository_root, toolchain)?;
    let dependency_closure =
        verify_dependency_closure(repository_root, &cargo_sandbox, &closure, &specification)?;
    fetch_dependency_sources(repository_root, &cargo_sandbox, &specification)?;
    cargo_sandbox.assert_empty_target()?;
    let build_output_authority = BuildOutputAuthority::open(&cargo_sandbox.target_dir)?;
    let xtask_binary = prepare_cargo_artifact(
        repository_root,
        &cargo_sandbox,
        &dependency_closure,
        &specification,
        XTASK_PACKAGE,
        Some("repository-input-candidate"),
        CargoArtifactTarget::Binary(XTASK_CARGO_TARGET),
    )?;
    let collector_test = prepare_cargo_artifact(
        repository_root,
        &cargo_sandbox,
        &dependency_closure,
        &specification,
        &specification.collector_gate.package,
        Some(&specification.collector_gate.feature),
        CargoArtifactTarget::IntegrationTest(&specification.collector_gate.test_target),
    )?;
    let probe_test = prepare_cargo_artifact(
        repository_root,
        &cargo_sandbox,
        &dependency_closure,
        &specification,
        &specification.runner.probe_package,
        None,
        CargoArtifactTarget::LibraryTest(PRODUCTION_PROBE_CARGO_TARGET),
    )?;
    let retained_build_outputs = build_output_authority.capture(
        &specification.closure.build_outputs,
        &[&xtask_binary, &collector_test, &probe_test],
    )?;
    let prepared_executables =
        observe_prepared_executables(&cargo_sandbox.target_dir, &[&collector_test, &probe_test])?;
    run_collector_gate(
        repository_root,
        &cargo_sandbox,
        &collector_test,
        &specification,
    )?;
    verify_prepared_executables(&prepared_executables)?;
    let probe_output =
        run_production_probe(repository_root, &cargo_sandbox, &probe_test, &specification)?;
    verify_prepared_executables(&prepared_executables)?;
    let (payload, report) =
        parse_production_probe_stdout(&probe_output, &specification.report_contract.prefix)?;
    let evidence_summary = validate_production_report(&specification, &payload, &report)?;
    build_output_authority.verify_unchanged(&retained_build_outputs)?;
    cargo_sandbox.verify_unchanged()?;
    repository
        .verify_unchanged(&closure)
        .map_err(|error| format!("U4c production inputs changed during measurement: {error}"))?;

    Ok(json!({
        "evidence_scope": PRODUCTION_SCOPE,
        "u4c_contract_verified": true,
        "qualifies_u4d_exit": evidence_summary.qualifies_u4d_exit,
        "qualifies_u4_exit": false,
        "specification_sha256": specification.specification_sha256,
        "report_sha256": specification.report_contract.sha256,
        "closure": {
            "roots": closure_summary.root_count,
            "files": closure_summary.file_count,
            "worktree_premise": specification.closure.worktree_premise,
            "build_outputs": build_output_summary(&retained_build_outputs),
        },
        "coverage": {
            "required_entry_points": specification.coverage.required_entry_points,
            "mapped_entry_points": specification.coverage.mapped_entry_points.len(),
            "declared_private_tracer_mapping_complete": evidence_summary.declared_private_tracer_mapping_complete,
        },
        "report": {
            "schema_id": report.schema_id,
            "schema_version": report.schema_version,
            "report_revision": report.report_revision,
            "scenario_set_id": report.scenario_set_id,
            "scale_scenarios": report.scale_scenarios.len(),
            "validation_faults": report.validation_faults.len(),
            "pending_scene_activation": report.pending_scene_activation.scenario_id.as_str(),
            "protected_terminal_activation": report.protected_terminal_activation.scenario_id.as_str(),
            "minimal_fallback": report.minimal_fallback.scenario_id.as_str(),
        },
    }))
}

impl ProductionSpecification {
    fn parse(bytes: &[u8]) -> Result<Self, String> {
        let contents = std::str::from_utf8(bytes)
            .map_err(|error| format!("U4c production specification must be UTF-8: {error}"))?;
        let specification = toml::from_str::<Self>(contents)
            .map_err(|error| format!("failed to parse U4c production specification: {error}"))?;
        specification.validate_static()?;
        Ok(specification)
    }

    fn validate_static(&self) -> Result<(), String> {
        if self.schema_version != SPECIFICATION_SCHEMA_VERSION {
            return Err(format!(
                "unsupported U4c production schema_version {}; expected {SPECIFICATION_SCHEMA_VERSION}",
                self.schema_version
            ));
        }
        if self.evidence_scope != PRODUCTION_SCOPE {
            return Err(format!(
                "U4c production evidence_scope must be {PRODUCTION_SCOPE:?}, found {:?}",
                self.evidence_scope
            ));
        }
        if self.runner.probe_package != PRODUCTION_PROBE_PACKAGE
            || self.runner.probe_target != PRODUCTION_PROBE_TARGET
            || self.runner.jobs != 1
            || self.runner.test_threads != 1
        {
            return Err(
                "U4c production runner must pin the canonical core probe with one Cargo job and one test thread"
                    .to_owned(),
            );
        }
        if self.report_contract.prefix != PRODUCTION_REPORT_PREFIX
            || self.report_contract.schema_id != "baozi.u4c.production-report"
            || self.report_contract.schema_version != PRODUCTION_REPORT_SCHEMA_VERSION
            || self.report_contract.report_revision != PRODUCTION_REPORT_REVISION
            || self.report_contract.scenario_set_id != PRODUCTION_SCENARIO_SET_ID
            || self.report_contract.payload_bytes == 0
            || !is_lowercase_sha256(&self.report_contract.sha256)
        {
            return Err(
                "U4c production report_contract is not the frozen production report contract"
                    .to_owned(),
            );
        }
        self.ledger_contract.validate()?;
        self.pending_scene_activation.validate()?;
        self.protected_terminal_activation.validate()?;
        self.minimal_fallback.validate()?;
        if self.identity.rustc_release.is_empty()
            || self.identity.rustup_toolchain != self.identity.rustc_release
            || self.identity.host.is_empty()
            || self.identity.target.is_empty()
            || self.identity.profile != "test"
            || self.identity.allocator != "allocation-counter-0.8.1"
            || !is_lowercase_sha256(&self.identity.rustc_verbose_sha256)
            || !is_lowercase_sha256(&self.identity.cargo_verbose_sha256)
            || !is_lowercase_sha256(&self.identity.libstd_sha256)
            || self.identity.toolchain_file_count == 0
            || !is_lowercase_sha256(&self.identity.toolchain_sha256)
        {
            return Err(
                "U4c production identity must pin the exact toolchain closure, test profile, and allocator identity"
                    .to_owned(),
            );
        }
        if self.identity.host != self.identity.target
            || !SUPPORTED_REPOSITORY_TARGETS.contains(&self.identity.target.as_str())
        {
            return Err(
                "U4c production identity must pin one supported native repository-capture target"
                    .to_owned(),
            );
        }
        if self.closure.worktree_premise != TRUSTED_STATIC_WORKTREE {
            return Err(format!(
                "U4c production closure worktree_premise must be {TRUSTED_STATIC_WORKTREE:?}"
            ));
        }
        self.closure.limits.validate()?;
        self.closure.build_outputs.validate_static()?;
        validate_source_trees(&self.closure.source_trees)?;
        validate_fixed_inputs(&self.closure.fixed_inputs)?;
        validate_packages(&self.closure.packages)?;
        validate_package_references("build_scripts", &self.closure.build_scripts)?;
        validate_package_references("proc_macros", &self.closure.proc_macros)?;
        validate_generated_outputs(&self.closure.generated_outputs)?;
        let required_roots = required_capture_root_count(&self.closure)?;
        if self.closure.limits.max_roots < required_roots {
            return Err(format!(
                "U4c production closure limits.max_roots must admit {required_roots} declared roots"
            ));
        }
        if self.collector_gate.package != COLLECTOR_PACKAGE
            || self.collector_gate.test_target != COLLECTOR_TEST_TARGET
            || self.collector_gate.feature != COLLECTOR_FEATURE
        {
            return Err(
                "U4c production collector_gate must pin the sealed collector test target"
                    .to_owned(),
            );
        }
        validate_semantic_vectors(&self.semantic_vectors)?;
        validate_mutations(&self.mutations)?;
        validate_scenarios(&self.scenarios)?;
        obj::validate_specification(&self.obj_capability)?;
        gltf::validate_specification(&self.gltf_capability, &self.ledger_contract)?;
        profile::validate_specification(&self.profile_closure)?;
        if self.measurement.scale_completed_lookups != 7
            || self.measurement.scale_rejected_lookups != 1
            || self.measurement.scale_lookup_events != 50
            || self.measurement.scale_member_probes != 6
            || self.measurement.max_lookup_events != 8
            || self.measurement.max_member_probes != 1
            || self.measurement.hot_view_max_allocations != 0
            || self.measurement.hot_view_max_element_decodes != 0
            || self.measurement.hot_view_max_payload_copy_bytes != 0
            || self.measurement.hot_view_max_payload_backing_allocations != 0
        {
            return Err(
                "U4c production measurement limits must preserve the frozen hot-view bounds"
                    .to_owned(),
            );
        }
        validate_coverage(
            &self.coverage,
            &self.obj_capability,
            &self.gltf_capability,
            &self.profile_closure,
            &self.pending_scene_activation,
        )?;

        Ok(())
    }
}

impl LedgerContractSpecification {
    fn validate(&self) -> Result<(), String> {
        if self.ordinary_semantic_id_hex != ORDINARY_DIMENSION_SEMANTIC_ID_HEX
            || self.failure_terminal_semantic_id_hex != FAILURE_TERMINAL_DIMENSION_SEMANTIC_ID_HEX
            || self.ordinary_semantic_id_hex == self.failure_terminal_semantic_id_hex
            || self.rich_failure_protected_attempted_work == 0
            || self.rich_failure_protected_attempted_work
                != self.rich_failure_protected_consumed_work
            || self.minimal_fallback_protected_attempted_work != 0
            || self.minimal_fallback_protected_consumed_work != 0
        {
            return Err(
                "U4c production ledger_contract must freeze the ordinary/protected vocabulary, an activated rich-failure prefix, and a dormant minimal-fallback prefix"
                    .to_owned(),
            );
        }
        Ok(())
    }
}

impl MinimalFallbackSpecification {
    fn validate(&self) -> Result<(), String> {
        if self.scenario_id != MINIMAL_FALLBACK_SCENARIO_ID
            || self.fallback_code == 0
            || self.terminal_failure_class != "ledger"
            || self.terminal_failure_code == 0
            || self.fallback_output_count != 1
        {
            return Err(
                "U4c production minimal_fallback must freeze the deterministic ledger rejection and its single bounded output"
                    .to_owned(),
            );
        }
        Ok(())
    }
}

impl ProtectedTerminalActivationSpecification {
    fn validate(&self) -> Result<(), String> {
        if self.scenario_id != PROTECTED_TERMINAL_ACTIVATION_SCENARIO_ID
            || self.terminal_failure_class != "ledger"
            || self.terminal_failure_code == 0
            || self.retained_evidence_blocks != 1
        {
            return Err(
                "U4c production protected_terminal_activation must freeze one ordinary-exhaustion rich failure"
                    .to_owned(),
            );
        }
        Ok(())
    }
}

impl PendingSceneActivationSpecification {
    fn validate(&self) -> Result<(), String> {
        if self.entry_point != PENDING_SCENE_ACTIVATION_ENTRY_POINT
            || self.scenario_id != PENDING_SCENE_ACTIVATION_SCENARIO_ID
            || self.canonical_block_count != PENDING_SCENE_ACTIVATION_CANONICAL_BLOCKS
            || self.outcome_evidence_block_count != PENDING_SCENE_ACTIVATION_OUTCOME_EVIDENCE_BLOCKS
            || self.new_block_count != PENDING_SCENE_ACTIVATION_NEW_BLOCKS
            || self.semantic_cells != PENDING_SCENE_ACTIVATION_SEMANTIC_CELLS
            || self.references != PENDING_SCENE_ACTIVATION_REFERENCES
            || self.producer_events != PENDING_SCENE_ACTIVATION_PRODUCER_EVENTS
            || self.max_payload_copy_bytes == 0
        {
            return Err(
                "U4d pending_scene_activation must freeze the bounded private activation tracer"
                    .to_owned(),
            );
        }
        Ok(())
    }

    fn matches_mapping(&self, mapping: &CoverageMappingSpecification) -> bool {
        mapping.entry_point == self.entry_point && mapping.scenario_id == self.scenario_id
    }
}

impl CaptureLimitsSpecification {
    fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("max_roots", self.max_roots),
            ("max_entries", self.max_entries),
            ("max_files", self.max_files),
            ("max_directories", self.max_directories),
            ("max_depth", self.max_depth),
            ("max_path_bytes", self.max_path_bytes),
            ("max_file_bytes", self.max_file_bytes),
            ("max_total_bytes", self.max_total_bytes),
            ("max_work", self.max_work),
        ] {
            if value == 0 {
                return Err(format!(
                    "U4c production closure limits.{name} must be nonzero"
                ));
            }
        }
        Ok(())
    }
}

fn validate_source_trees(values: &[SourceTreeSpecification]) -> Result<(), String> {
    let paths = values
        .iter()
        .map(|value| value.path.as_str())
        .collect::<Vec<_>>();
    validate_exact_list("closure.source_trees", &paths, REQUIRED_SOURCE_TREES)?;
    for value in values {
        validate_repository_path(&value.path, "U4c production source tree")?;
        if value.file_count == 0 || !is_lowercase_sha256(&value.sha256) {
            return Err(format!(
                "U4c production source tree {} must declare a positive file count and lowercase SHA-256",
                value.path
            ));
        }
    }
    Ok(())
}

fn validate_fixed_inputs(values: &[FixedInputSpecification]) -> Result<(), String> {
    let paths = values
        .iter()
        .map(|value| value.path.as_str())
        .collect::<Vec<_>>();
    validate_exact_list("closure.fixed_inputs", &paths, REQUIRED_FIXED_INPUTS)?;
    for value in values {
        validate_repository_path(&value.path, "U4c production fixed input")?;
        if !is_lowercase_sha256(&value.sha256) {
            return Err(format!(
                "U4c production fixed input {} must declare a lowercase SHA-256",
                value.path
            ));
        }
    }
    Ok(())
}

fn validate_packages(values: &[PackageSpecification]) -> Result<(), String> {
    if values.is_empty() {
        return Err("U4c production closure must declare resolved packages".to_owned());
    }
    let mut previous = None;
    for value in values {
        if value.name.is_empty() || value.version.is_empty() {
            return Err("U4c production package identity must not be empty".to_owned());
        }
        match (&value.source, &value.checksum, &value.repository_manifest) {
            (Some(source), Some(checksum), None)
                if !source.is_empty() && is_lowercase_sha256(checksum) => {}
            (None, None, Some(manifest)) => {
                validate_repository_path(manifest, "U4c production local package manifest")?;
            }
            _ => {
                return Err(format!(
                    "U4c production package {}@{} must declare either registry source/checksum or one repository manifest",
                    value.name, value.version
                ));
            }
        }
        let key = format!(
            "{}\u{0}{}\u{0}{}\u{0}{}",
            value.name,
            value.version,
            value.source.as_deref().unwrap_or_default(),
            value.repository_manifest.as_deref().unwrap_or_default()
        );
        if previous
            .as_deref()
            .is_some_and(|prior| prior >= key.as_str())
        {
            return Err(
                "U4c production closure packages must be strictly sorted by identity".to_owned(),
            );
        }
        previous = Some(key);
    }
    Ok(())
}

fn validate_package_references(label: &str, values: &[PackageReference]) -> Result<(), String> {
    let mut previous = None;
    for value in values {
        if value.name.is_empty() || value.version.is_empty() {
            return Err(format!(
                "U4c production {label} package identity must not be empty"
            ));
        }
        let key = format!("{}\u{0}{}", value.name, value.version);
        if previous
            .as_deref()
            .is_some_and(|prior| prior >= key.as_str())
        {
            return Err(format!(
                "U4c production {label} must be strictly sorted by package identity"
            ));
        }
        previous = Some(key);
    }
    Ok(())
}

fn validate_generated_outputs(values: &[GeneratedOutputSpecification]) -> Result<(), String> {
    let mut paths = BTreeSet::new();
    for value in values {
        validate_repository_path(&value.path, "U4c production generated output")?;
        if value.generator.is_empty() || !is_lowercase_sha256(&value.sha256) {
            return Err(format!(
                "U4c production generated output {} must declare generator and lowercase SHA-256",
                value.path
            ));
        }
        if !paths.insert(value.path.as_str()) {
            return Err(format!(
                "U4c production generated output {} is duplicated",
                value.path
            ));
        }
    }
    Ok(())
}

fn required_capture_root_count(closure: &ClosureSpecification) -> Result<u64, String> {
    let source_and_fixed_roots = closure
        .source_trees
        .len()
        .checked_add(closure.fixed_inputs.len())
        .ok_or_else(|| "U4c production declared capture root count overflowed".to_owned())?;
    let mut root_count = u64::try_from(source_and_fixed_roots)
        .map_err(|_| "U4c production declared capture root count overflowed".to_owned())?
        .checked_add(1)
        .ok_or_else(|| "U4c production declared capture root count overflowed".to_owned())?;

    let mut standalone_generated_outputs = Vec::new();
    for generated in &closure.generated_outputs {
        if generated_output_requires_root(closure, &generated.path)? {
            standalone_generated_outputs.push(generated.path.as_str());
            root_count = root_count.checked_add(1).ok_or_else(|| {
                "U4c production declared capture root count overflowed".to_owned()
            })?;
        }
    }
    standalone_generated_outputs.sort_unstable();
    for pair in standalone_generated_outputs.windows(2) {
        if repository_path_is_prefix(pair[0], pair[1]) {
            return Err(format!(
                "U4c production generated outputs {} and {} would overlap capture roots",
                pair[0], pair[1]
            ));
        }
    }
    Ok(root_count)
}

fn generated_output_requires_root(
    closure: &ClosureSpecification,
    generated_path: &str,
) -> Result<bool, String> {
    if generated_path == SPECIFICATION_PATH
        || closure
            .fixed_inputs
            .iter()
            .any(|input| input.path == generated_path)
    {
        return Err(format!(
            "U4c production generated output {generated_path} duplicates a fixed capture root"
        ));
    }
    for source_tree in &closure.source_trees {
        if source_tree.path == generated_path {
            return Err(format!(
                "U4c production generated output {generated_path} duplicates a source-tree root"
            ));
        }
        if repository_path_is_prefix(&source_tree.path, generated_path) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn repository_path_is_prefix(prefix: &str, path: &str) -> bool {
    path.strip_prefix(prefix)
        .and_then(|suffix| suffix.strip_prefix('/'))
        .is_some_and(|suffix| !suffix.is_empty())
}

fn validate_semantic_vectors(values: &[SemanticVectorSpecification]) -> Result<(), String> {
    let identifiers = values
        .iter()
        .map(|value| value.scenario_id.as_str())
        .collect::<Vec<_>>();
    validate_exact_list("semantic_vectors", &identifiers, REQUIRED_SCALE_SCENARIOS)?;
    for value in values {
        if value.observed_bytes == 0 || !is_lowercase_sha256(&value.sha256) {
            return Err(format!(
                "U4c production semantic vector {} must declare bytes and lowercase SHA-256",
                value.scenario_id
            ));
        }
    }
    Ok(())
}

fn validate_mutations(values: &[MutationSpecification]) -> Result<(), String> {
    let identifiers = values
        .iter()
        .map(|value| value.scenario_id.as_str())
        .collect::<Vec<_>>();
    validate_exact_list("mutations", &identifiers, REQUIRED_MUTATION_SCENARIOS)?;
    for value in values {
        if value.field_class.is_empty()
            || value.transcript_bytes == 0
            || !is_lowercase_sha256(&value.transcript_sha256)
            || value.failure_code == 0
            || value.retained_evidence_blocks == 0
        {
            return Err(format!(
                "U4c production mutation {} must declare class, failure code, and evidence blocks",
                value.scenario_id
            ));
        }
    }
    Ok(())
}

fn validate_scenarios(values: &[ScaleScenarioSpecification]) -> Result<(), String> {
    let identifiers = values
        .iter()
        .map(|value| value.scenario_id.as_str())
        .collect::<Vec<_>>();
    validate_exact_list("scenarios", &identifiers, REQUIRED_SCALE_SCENARIOS)?;
    for value in values {
        if value.image_count == 0
            || value.canonical_block_count == 0
            || value.blocks == 0
            || value.target_live_retained_bytes == 0
            || value.target_live_retained_allocations == 0
        {
            return Err(format!(
                "U4c production scenario {} must declare nonzero scale and retained values",
                value.scenario_id
            ));
        }
        let blocks = value.blocks;
        let producer_limit = value
            .semantic_cells
            .checked_add(value.references)
            .and_then(|total| total.checked_add(blocks.saturating_mul(3)))
            .ok_or_else(|| {
                format!(
                    "U4c production scenario {} producer budget overflowed",
                    value.scenario_id
                )
            })?;
        if value.producer_events > producer_limit {
            return Err(format!(
                "U4c production scenario {} exceeds the producer event inequality",
                value.scenario_id
            ));
        }
    }
    Ok(())
}

fn validate_coverage(
    coverage: &CoverageSpecification,
    obj_capability: &obj::ObjCapabilitySpecification,
    gltf_capability: &gltf::GltfCapabilitySpecification,
    profile_closure: &profile::ProfileClosureSpecification,
    pending_scene_activation: &PendingSceneActivationSpecification,
) -> Result<(), String> {
    let required = coverage
        .required_entry_points
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    validate_exact_list(
        "coverage.required_entry_points",
        &required,
        REQUIRED_COVERAGE_ENTRY_POINTS,
    )?;
    let [
        obj_mapping,
        gltf_mapping,
        profile_mapping,
        pending_activation_mapping,
        consumer_mappings @ ..,
    ] = coverage.mapped_entry_points.as_slice()
    else {
        return Err(
            "U4c production coverage must map the admitted producer capabilities".to_owned(),
        );
    };
    if !obj_capability.matches_mapping(obj_mapping) {
        return Err(
            "U4c production OBJ coverage mapping must bind the typed path-bound scenario"
                .to_owned(),
        );
    }
    if !gltf_capability.matches_mapping(gltf_mapping) {
        return Err(
            "U4c production glTF coverage mapping must bind the backend-free five-domain scenario"
                .to_owned(),
        );
    }
    if profile_mapping.entry_point != profile_closure.entry_point
        || profile_mapping.scenario_id != profile_closure.scenario_id
    {
        return Err(
            "U4c production profile-closure coverage mapping must bind the canonical profile scenario"
                .to_owned(),
        );
    }
    if !pending_scene_activation.matches_mapping(pending_activation_mapping) {
        return Err(
            "U4d pending activation coverage mapping must bind the private canonical activation scenario"
                .to_owned(),
        );
    }
    if consumer_mappings.len() != MEASURED_PRIVATE_TRACER_COVERAGE.len()
        || consumer_mappings
            .iter()
            .zip(MEASURED_PRIVATE_TRACER_COVERAGE)
            .any(|(mapping, (entry_point, scenario_id))| {
                mapping.entry_point != *entry_point || mapping.scenario_id != *scenario_id
            })
    {
        return Err(
            "U4d private tracer coverage mapping must bind inspection, snapshot, unchanged-view, and changed-scene-edit evidence"
                .to_owned(),
        );
    }
    Ok(())
}

fn validate_exact_list(label: &str, actual: &[&str], expected: &[&str]) -> Result<(), String> {
    if actual != expected {
        return Err(format!(
            "U4c production {label} must equal {expected:?}, found {actual:?}"
        ));
    }
    Ok(())
}

fn validate_repository_path(path: &str, label: &str) -> Result<(), String> {
    if path.is_empty()
        || path.contains('\\')
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains("//")
        || !path.is_ascii()
    {
        return Err(format!(
            "{label} path {path:?} must be an ASCII forward-slash repository-relative path"
        ));
    }
    if Path::new(path)
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "{label} path {path:?} must not contain a prefix, root, or dot component"
        ));
    }
    Ok(())
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[cfg(feature = "repository-input-candidate")]
fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(feature = "repository-input-candidate")]
struct ClosureSummary {
    root_count: u64,
    file_count: u64,
}

#[cfg(feature = "repository-input-candidate")]
fn build_output_summary(capture: &RetainedBuildOutputCapture) -> Value {
    json!({
        "build_script_records": capture.build_script_records(),
        "out_directories": capture.out_directories(),
        "files": capture.files(),
        "directories": capture.directories(),
        "total_bytes": capture.total_bytes(),
        "transcript_sha256": capture.transcript_sha256(),
        "tree_sha256": capture.tree_sha256(),
    })
}

#[cfg(feature = "repository-input-candidate")]
fn capture_limits(
    limits: &CaptureLimitsSpecification,
) -> Result<baozi_io::RepositoryLimits, String> {
    baozi_io::RepositoryLimitSpec {
        max_roots: limits.max_roots,
        max_entries: limits.max_entries,
        max_files: limits.max_files,
        max_directories: limits.max_directories,
        max_depth: limits.max_depth,
        max_path_bytes: limits.max_path_bytes,
        max_file_bytes: limits.max_file_bytes,
        max_total_bytes: limits.max_total_bytes,
        max_work: limits.max_work,
    }
    .validate()
    .map_err(|error| format!("invalid U4c repository capture limits: {error}"))
}

#[cfg(feature = "repository-input-candidate")]
fn repository_file_root(path: &str) -> Result<baozi_io::RepositoryRootSpec, String> {
    baozi_io::RepositoryPath::new(path)
        .map(baozi_io::RepositoryRootSpec::File)
        .map_err(|error| format!("invalid U4c repository file root {path:?}: {error}"))
}

#[cfg(feature = "repository-input-candidate")]
fn repository_tree_root(path: &str) -> Result<baozi_io::RepositoryRootSpec, String> {
    baozi_io::RepositoryPath::new(path)
        .map(baozi_io::RepositoryRootSpec::Tree)
        .map_err(|error| format!("invalid U4c repository tree root {path:?}: {error}"))
}

#[cfg(feature = "repository-input-candidate")]
fn captured_file_bytes<'capture>(
    capture: &'capture baozi_io::RepositoryCapture,
    path: &str,
) -> Result<&'capture [u8], String> {
    let files = capture
        .files()
        .iter()
        .filter(|file| file.path().as_str() == path)
        .collect::<Vec<_>>();
    match files.as_slice() {
        [file] => Ok(file.bytes()),
        [] => Err(format!(
            "U4c repository capture omitted required file {path}"
        )),
        _ => Err(format!(
            "U4c repository capture duplicated required file {path}"
        )),
    }
}

fn verify_bootstrap_specification_bytes(
    bootstrap_bytes: &[u8],
    closure_bytes: &[u8],
) -> Result<(), String> {
    if bootstrap_bytes == closure_bytes {
        Ok(())
    } else {
        Err(
            "U4c production specification changed between bootstrap and complete closure capture"
                .to_owned(),
        )
    }
}

#[cfg(feature = "repository-input-candidate")]
fn capture_declared_closure(
    repository: &baozi_io::RepositoryRoot,
    specification: &ProductionSpecification,
) -> Result<baozi_io::RepositoryCapture, String> {
    let mut roots = Vec::with_capacity(
        1 + specification.closure.source_trees.len()
            + specification.closure.fixed_inputs.len()
            + specification.closure.generated_outputs.len(),
    );
    roots.push(repository_file_root(SPECIFICATION_PATH)?);
    for source_tree in &specification.closure.source_trees {
        roots.push(repository_tree_root(&source_tree.path)?);
    }
    for fixed_input in &specification.closure.fixed_inputs {
        roots.push(repository_file_root(&fixed_input.path)?);
    }
    for generated in &specification.closure.generated_outputs {
        if generated_output_requires_root(&specification.closure, &generated.path)? {
            roots.push(repository_file_root(&generated.path)?);
        }
    }
    let limits = capture_limits(&specification.closure.limits)?;
    repository
        .capture(roots, limits)
        .map_err(|error| format!("failed to capture U4c production closure: {error}"))
}

#[cfg(feature = "repository-input-candidate")]
fn verify_captured_closure(
    capture: &baozi_io::RepositoryCapture,
    specification: &ProductionSpecification,
) -> Result<ClosureSummary, String> {
    let mut captured = BTreeMap::new();
    for file in capture.files() {
        let path = file.path().as_str();
        if captured.insert(path, file.bytes()).is_some() {
            return Err(format!(
                "U4c production closure captured duplicate file {path}"
            ));
        }
    }

    let mut expected_paths = BTreeSet::new();
    expected_paths.insert(SPECIFICATION_PATH);
    for fixed_input in &specification.closure.fixed_inputs {
        expected_paths.insert(fixed_input.path.as_str());
        let bytes = captured_file_bytes(capture, &fixed_input.path)?;
        let actual = sha256_hex(bytes);
        if actual != fixed_input.sha256 {
            return Err(format!(
                "U4c production fixed input {} drifted: expected {}, found {actual}",
                fixed_input.path, fixed_input.sha256
            ));
        }
    }
    for source_tree in &specification.closure.source_trees {
        let prefix = format!("{}/", source_tree.path);
        let mut files = captured
            .iter()
            .filter(|(path, _)| path.starts_with(&prefix))
            .map(|(path, bytes)| (*path, *bytes))
            .collect::<Vec<_>>();
        files.sort_unstable_by_key(|(path, _)| *path);
        if files.is_empty() {
            return Err(format!(
                "U4c production source tree {} captured no files",
                source_tree.path
            ));
        }
        let file_count = u64::try_from(files.len())
            .map_err(|_| "U4c production source tree file count overflowed".to_owned())?;
        if file_count != source_tree.file_count {
            return Err(format!(
                "U4c production source tree {} file count drifted: expected {}, found {file_count}",
                source_tree.path, source_tree.file_count
            ));
        }
        let mut digest = Sha256::new();
        for (path, bytes) in files {
            if path == "build.rs" || path.ends_with("/build.rs") {
                return Err(format!(
                    "U4c production local build script {path} is undeclared"
                ));
            }
            expected_paths.insert(path);
            digest.update(path.as_bytes());
            digest.update([0]);
            digest.update(sha256_hex(bytes).as_bytes());
            digest.update(b"\n");
        }
        let actual = format!("{:x}", digest.finalize());
        if actual != source_tree.sha256 {
            return Err(format!(
                "U4c production source tree {} drifted: expected {}, found {actual}",
                source_tree.path, source_tree.sha256
            ));
        }
    }
    for generated in &specification.closure.generated_outputs {
        expected_paths.insert(generated.path.as_str());
        let bytes = captured
            .get(generated.path.as_str())
            .copied()
            .ok_or_else(|| {
                format!(
                    "U4c production generated output {} was not captured",
                    generated.path
                )
            })?;
        let actual = sha256_hex(bytes);
        if actual != generated.sha256 {
            return Err(format!(
                "U4c production generated output {} drifted: expected {}, found {actual}",
                generated.path, generated.sha256
            ));
        }
    }
    let unexpected = captured
        .keys()
        .filter(|path| !expected_paths.contains(*path))
        .copied()
        .collect::<Vec<_>>();
    if !unexpected.is_empty() {
        return Err(format!(
            "U4c production closure captured undeclared inputs: {unexpected:?}"
        ));
    }

    let root_count = capture.usage().roots();
    let required_roots = required_capture_root_count(&specification.closure)?;
    if root_count != required_roots {
        return Err(format!(
            "U4c production closure captured {root_count} roots; expected {required_roots}"
        ));
    }

    Ok(ClosureSummary {
        root_count,
        file_count: capture.usage().files(),
    })
}

#[cfg(feature = "repository-input-candidate")]
struct CargoSandbox {
    root: PathBuf,
    repository_root: PathBuf,
    cargo_home: PathBuf,
    sandbox_home: PathBuf,
    temp_dir: PathBuf,
    target_dir: PathBuf,
    execution_directory: PathBuf,
    toolchain: VerifiedToolchain,
}

#[cfg(feature = "repository-input-candidate")]
impl CargoSandbox {
    fn create(repository_root: &Path, toolchain: VerifiedToolchain) -> Result<Self, String> {
        let execution_directory = cargo_execution_directory(repository_root)?;
        let root = create_cargo_sandbox_directory()?;
        let cargo_home = root.join("cargo-home");
        if let Err(error) = fs::create_dir(&cargo_home) {
            let _ = fs::remove_dir_all(&root);
            return Err(format!(
                "failed to create isolated U4c Cargo home {}: {error}",
                cargo_home.display()
            ));
        }
        let sandbox_home = root.join("home");
        if let Err(error) = fs::create_dir(&sandbox_home) {
            let _ = fs::remove_dir_all(&root);
            return Err(format!(
                "failed to create isolated U4c home {}: {error}",
                sandbox_home.display()
            ));
        }
        let temp_dir = root.join("temp");
        if let Err(error) = fs::create_dir(&temp_dir) {
            let _ = fs::remove_dir_all(&root);
            return Err(format!(
                "failed to create isolated U4c temporary directory {}: {error}",
                temp_dir.display()
            ));
        }
        let target_dir = root.join("target");
        if let Err(error) = fs::create_dir(&target_dir) {
            let _ = fs::remove_dir_all(&root);
            return Err(format!(
                "failed to create isolated U4c Cargo target directory {}: {error}",
                target_dir.display()
            ));
        }
        Ok(Self {
            root,
            repository_root: repository_root.to_path_buf(),
            cargo_home,
            sandbox_home,
            temp_dir,
            target_dir,
            execution_directory,
            toolchain,
        })
    }

    fn network_command(&self) -> Command {
        let mut command = Command::new(&self.toolchain.cargo);
        command.current_dir(&self.execution_directory);
        for (name, _) in env::vars_os() {
            let Some(name) = name.to_str() else {
                continue;
            };
            if is_sanitized_cargo_environment_name(name) {
                command.env_remove(name);
            }
        }
        command
            .env_remove("CARGO")
            .env("CARGO_CACHE_RUSTC_INFO", "0")
            .env("CARGO_HOME", &self.cargo_home)
            .env("CARGO_TARGET_DIR", &self.target_dir)
            .env("CARGO_BUILD_JOBS", "1")
            .env("RUSTC", &self.toolchain.rustc)
            .args(["--config", "build.jobs = 1"]);
        command
    }

    fn build_command(&self) -> Result<Command, String> {
        let path = deterministic_build_path(&self.toolchain.sysroot)?;
        let encoded_rustflags = deterministic_encoded_rustflags(
            &self.root,
            &self.repository_root,
            &self.toolchain.sysroot,
        )?;
        let mut command = Command::new(&self.toolchain.cargo);
        command
            .current_dir(&self.execution_directory)
            .env_clear()
            .env("CARGO_CACHE_RUSTC_INFO", "0")
            .env("CARGO_HOME", &self.cargo_home)
            .env("CARGO_TARGET_DIR", &self.target_dir)
            .env("CARGO_BUILD_JOBS", "1")
            .env("CARGO_ENCODED_RUSTFLAGS", encoded_rustflags)
            .env("CARGO_INCREMENTAL", "0")
            .env("CARGO_NET_OFFLINE", "true")
            .env("HOME", &self.sandbox_home)
            .env("LANG", "C")
            .env("LC_ALL", "C")
            .env("PATH", path)
            .env("RUSTC", &self.toolchain.rustc)
            .env("SOURCE_DATE_EPOCH", "0")
            .env("TEMP", &self.temp_dir)
            .env("TMP", &self.temp_dir)
            .env("TMPDIR", &self.temp_dir)
            .env("TZ", "UTC")
            .args(["--config", "build.jobs = 1"]);
        Ok(command)
    }

    fn prepared_test_command(
        &self,
        prepared: &PreparedCargoArtifact,
        working_directory: &Path,
    ) -> Result<Command, String> {
        let executable = prepared.executable();
        let relative = executable.strip_prefix(&self.target_dir).map_err(|_| {
            format!(
                "prepared U4c test executable {} escaped target directory {}",
                executable.display(),
                self.target_dir.display()
            )
        })?;
        if relative.as_os_str().is_empty()
            || !relative
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err(format!(
                "prepared U4c test executable {} is not target-relative",
                executable.display()
            ));
        }
        let metadata = fs::symlink_metadata(executable).map_err(|error| {
            format!(
                "failed to inspect prepared U4c test executable {}: {error}",
                executable.display()
            )
        })?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(format!(
                "prepared U4c test executable {} is not a regular non-symlink file",
                executable.display()
            ));
        }
        if !working_directory.is_dir() {
            return Err(format!(
                "prepared U4c test working directory does not exist: {}",
                working_directory.display()
            ));
        }

        let path = deterministic_build_path(&self.toolchain.sysroot)?;
        let mut command = Command::new(executable);
        command
            .current_dir(working_directory)
            .env_clear()
            .env("HOME", &self.sandbox_home)
            .env("LANG", "C")
            .env("LC_ALL", "C")
            .env("PATH", path)
            .env("SOURCE_DATE_EPOCH", "0")
            .env("TEMP", &self.temp_dir)
            .env("TMP", &self.temp_dir)
            .env("TMPDIR", &self.temp_dir)
            .env("TZ", "UTC");
        Ok(command)
    }

    fn assert_empty_target(&self) -> Result<(), String> {
        let mut entries = fs::read_dir(&self.target_dir).map_err(|error| {
            format!(
                "failed to inspect isolated U4c target directory {}: {error}",
                self.target_dir.display()
            )
        })?;
        if entries
            .next()
            .transpose()
            .map_err(|error| {
                format!(
                    "failed to inspect isolated U4c target directory {}: {error}",
                    self.target_dir.display()
                )
            })?
            .is_some()
        {
            return Err(format!(
                "U4c production target directory was not empty before admitted compilation: {}",
                self.target_dir.display()
            ));
        }
        Ok(())
    }

    fn verify_unchanged(&self) -> Result<(), String> {
        verify_cargo_configuration_boundary(&self.execution_directory)?;
        self.toolchain.verify_unchanged()
    }
}

#[cfg(feature = "repository-input-candidate")]
fn deterministic_build_path(sysroot: &Path) -> Result<std::ffi::OsString, String> {
    let paths = [
        sysroot.join("bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
    ];
    env::join_paths(paths)
        .map_err(|error| format!("failed to construct deterministic U4c build PATH: {error}"))
}

#[cfg(feature = "repository-input-candidate")]
fn deterministic_encoded_rustflags(
    sandbox_root: &Path,
    repository_root: &Path,
    sysroot: &Path,
) -> Result<std::ffi::OsString, String> {
    let mappings = [
        ("Cargo sandbox root", sandbox_root, "/__baozi_u4c_sandbox__"),
        (
            "Baozi repository root",
            repository_root,
            "/__baozi_repository__",
        ),
        ("Rust toolchain root", sysroot, "/__baozi_rust_toolchain__"),
    ];
    let mut flags = Vec::with_capacity(mappings.len());
    for (label, source, destination) in mappings {
        if !source.is_absolute() {
            return Err(format!(
                "{label} must be absolute, found {}",
                source.display()
            ));
        }
        let source = source
            .to_str()
            .ok_or_else(|| format!("{label} {} is not valid UTF-8", source.display()))?;
        if source.contains(['=', '\u{1f}']) {
            return Err(format!(
                "{label} {source:?} cannot be represented in encoded rustflags"
            ));
        }
        flags.push(format!("--remap-path-prefix={source}={destination}"));
    }
    Ok(std::ffi::OsString::from(flags.join("\u{1f}")))
}

#[cfg(feature = "repository-input-candidate")]
impl Drop for CargoSandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(feature = "repository-input-candidate")]
struct PreparedExecutableObservation {
    path: PathBuf,
    sha256: String,
}

#[cfg(feature = "repository-input-candidate")]
fn observe_prepared_executables(
    target_dir: &Path,
    prepared_artifacts: &[&PreparedCargoArtifact],
) -> Result<Vec<PreparedExecutableObservation>, String> {
    let mut paths = BTreeSet::new();
    let mut observations = Vec::with_capacity(prepared_artifacts.len());
    for prepared in prepared_artifacts {
        let path = prepared.executable();
        let relative = path.strip_prefix(target_dir).map_err(|_| {
            format!(
                "prepared U4c executable {} escaped target directory {}",
                path.display(),
                target_dir.display()
            )
        })?;
        if relative.as_os_str().is_empty()
            || !relative
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err(format!(
                "prepared U4c executable {} is not target-relative",
                path.display()
            ));
        }
        let metadata = fs::symlink_metadata(path).map_err(|error| {
            format!(
                "failed to inspect prepared U4c executable {}: {error}",
                path.display()
            )
        })?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(format!(
                "prepared U4c executable {} is not a regular non-symlink file",
                path.display()
            ));
        }
        if !paths.insert(path.to_path_buf()) {
            return Err(format!(
                "U4c Cargo preparation reused executable {} for distinct artifacts",
                path.display()
            ));
        }
        observations.push(PreparedExecutableObservation {
            path: path.to_path_buf(),
            sha256: sha256_file(path)?,
        });
    }
    Ok(observations)
}

#[cfg(feature = "repository-input-candidate")]
fn verify_prepared_executables(
    observations: &[PreparedExecutableObservation],
) -> Result<(), String> {
    for observation in observations {
        let actual = sha256_file(&observation.path)?;
        if actual != observation.sha256 {
            return Err(format!(
                "prepared U4c executable changed during measurement: {}",
                observation.path.display()
            ));
        }
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
fn cargo_execution_directory(repository_root: &Path) -> Result<PathBuf, String> {
    let root = repository_root
        .ancestors()
        .last()
        .filter(|path| path.has_root())
        .ok_or_else(|| {
            format!(
                "U4c production repository root must be absolute to isolate Cargo configuration: {}",
                repository_root.display()
            )
        })?;
    verify_cargo_configuration_boundary(root)?;
    Ok(root.to_path_buf())
}

#[cfg(feature = "repository-input-candidate")]
fn verify_cargo_configuration_boundary(root: &Path) -> Result<(), String> {
    for configuration in [".cargo/config.toml", ".cargo/config"] {
        let path = root.join(configuration);
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                return Err(format!(
                    "U4c production refuses ambient Cargo configuration at {}; move it outside the filesystem root",
                    path.display()
                ));
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "failed to inspect U4c Cargo configuration boundary {}: {error}",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
fn is_sanitized_cargo_environment_name(name: &str) -> bool {
    let normalized = name.to_ascii_uppercase();
    normalized.starts_with("CARGO_")
        || matches!(
            normalized.as_str(),
            "RUSTFLAGS"
                | "RUSTDOCFLAGS"
                | "RUSTC"
                | "RUSTC_BOOTSTRAP"
                | "RUSTC_WRAPPER"
                | "RUSTC_WORKSPACE_WRAPPER"
                | "RUSTUP_HOME"
                | "RUSTUP_TOOLCHAIN"
                | "LD_PRELOAD"
                | "LD_LIBRARY_PATH"
                | "DYLD_INSERT_LIBRARIES"
                | "DYLD_LIBRARY_PATH"
        )
}

#[cfg(feature = "repository-input-candidate")]
fn create_cargo_sandbox_directory() -> Result<PathBuf, String> {
    let parent = env::temp_dir();
    for _ in 0..64 {
        let sequence = CARGO_SANDBOX_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!("baozi-u4c-{}-{sequence}", process::id()));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "failed to create U4c Cargo sandbox {}: {error}",
                    path.display()
                ));
            }
        }
    }
    Err("failed to reserve a unique U4c Cargo sandbox directory".to_owned())
}

#[cfg(feature = "repository-input-candidate")]
struct VerifiedToolchain {
    cargo: PathBuf,
    rustc: PathBuf,
    sysroot: PathBuf,
    target: String,
    file_count: u64,
    sha256: String,
}

#[cfg(feature = "repository-input-candidate")]
impl VerifiedToolchain {
    fn admit(identity: &IdentitySpecification) -> Result<Self, String> {
        let rustc_proxy = find_path_executable("rustc")?;
        let mut sysroot_command = Command::new(&rustc_proxy);
        for (name, _) in env::vars_os() {
            let Some(name) = name.to_str() else {
                continue;
            };
            if is_sanitized_cargo_environment_name(name) {
                sysroot_command.env_remove(name);
            }
        }
        let sysroot_output = sysroot_command
            .args(["--print", "sysroot"])
            .env("RUSTUP_TOOLCHAIN", &identity.rustup_toolchain)
            .output()
            .map_err(|error| {
                format!(
                    "failed to locate U4c toolchain through {}: {error}",
                    rustc_proxy.display()
                )
            })?;
        if !sysroot_output.status.success() {
            return Err(format!(
                "U4c rustc proxy failed to locate toolchain {} with {}: {}",
                identity.rustup_toolchain,
                sysroot_output.status,
                command_stderr(&sysroot_output.stderr)
            ));
        }
        let sysroot = std::str::from_utf8(&sysroot_output.stdout)
            .map_err(|error| format!("U4c rustc proxy emitted a non-UTF-8 sysroot: {error}"))?
            .trim();
        let sysroot = PathBuf::from(sysroot);
        if !sysroot.is_absolute() {
            return Err(format!(
                "U4c toolchain sysroot must be absolute, found {}",
                sysroot.display()
            ));
        }

        let cargo = toolchain_binary(&sysroot, "cargo")?;
        let rustc = toolchain_binary(&sysroot, "rustc")?;
        let rustc_verbose = run_tool_identity(&rustc, &["-vV"], "rustc")?;
        let cargo_verbose = run_tool_identity(&cargo, &["-Vv"], "Cargo")?;
        let rustc_text = std::str::from_utf8(&rustc_verbose)
            .map_err(|error| format!("U4c rustc -vV output was not UTF-8: {error}"))?;
        let release = metadata_line(rustc_text, "release")?;
        let host = metadata_line(rustc_text, "host")?;
        if release != identity.rustc_release || host != identity.host {
            return Err(format!(
                "U4c production toolchain drifted: expected {} on {}, found {release} on {host}",
                identity.rustc_release, identity.host
            ));
        }
        verify_digest("rustc -vV", &rustc_verbose, &identity.rustc_verbose_sha256)?;
        verify_digest("Cargo -Vv", &cargo_verbose, &identity.cargo_verbose_sha256)?;
        let libstd = find_unique_libstd(&sysroot, &identity.target)?;
        verify_file_digest("target libstd", &libstd, &identity.libstd_sha256)?;
        let (file_count, digest) = hash_toolchain_closure(&sysroot, &identity.target)?;
        if file_count != identity.toolchain_file_count || digest != identity.toolchain_sha256 {
            return Err(format!(
                "U4c production toolchain files drifted: expected {} files/{}, found {file_count}/{digest}",
                identity.toolchain_file_count, identity.toolchain_sha256
            ));
        }
        Ok(Self {
            cargo,
            rustc,
            sysroot,
            target: identity.target.clone(),
            file_count,
            sha256: digest,
        })
    }

    fn verify_unchanged(&self) -> Result<(), String> {
        let (file_count, sha256) = hash_toolchain_closure(&self.sysroot, &self.target)?;
        if file_count != self.file_count || sha256 != self.sha256 {
            return Err(format!(
                "U4c production toolchain changed during measurement: expected {} files/{}, found {file_count}/{sha256}",
                self.file_count, self.sha256
            ));
        }
        Ok(())
    }
}

#[cfg(feature = "repository-input-candidate")]
fn find_path_executable(name: &str) -> Result<PathBuf, String> {
    let path = env::var_os("PATH")
        .ok_or_else(|| format!("U4c production cannot locate {name} without PATH"))?;
    let file_name = format!("{name}{}", env::consts::EXE_SUFFIX);
    for directory in env::split_paths(&path) {
        let candidate = directory.join(&file_name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "U4c production could not locate {file_name} on PATH"
    ))
}

#[cfg(feature = "repository-input-candidate")]
fn toolchain_binary(sysroot: &Path, name: &str) -> Result<PathBuf, String> {
    let path = sysroot
        .join("bin")
        .join(format!("{name}{}", env::consts::EXE_SUFFIX));
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "U4c production toolchain omitted executable {}",
            path.display()
        ))
    }
}

#[cfg(feature = "repository-input-candidate")]
fn run_tool_identity(path: &Path, arguments: &[&str], label: &str) -> Result<Vec<u8>, String> {
    let output = Command::new(path)
        .args(arguments)
        .env_clear()
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .output()
        .map_err(|error| format!("failed to run U4c {label} identity command: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "U4c {label} identity command failed with {}: {}",
            output.status,
            command_stderr(&output.stderr)
        ));
    }
    Ok(output.stdout)
}

#[cfg(feature = "repository-input-candidate")]
fn verify_digest(label: &str, bytes: &[u8], expected: &str) -> Result<(), String> {
    let actual = sha256_hex(bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "U4c production {label} identity drifted: expected {expected}, found {actual}"
        ))
    }
}

#[cfg(feature = "repository-input-candidate")]
fn verify_file_digest(label: &str, path: &Path, expected: &str) -> Result<(), String> {
    let actual = sha256_file(path)?;
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "U4c production {label} identity drifted: expected {expected}, found {actual}"
        ))
    }
}

#[cfg(feature = "repository-input-candidate")]
fn hash_toolchain_closure(sysroot: &Path, target: &str) -> Result<(u64, String), String> {
    let mut files = vec![
        toolchain_binary(sysroot, "cargo")?,
        toolchain_binary(sysroot, "rustc")?,
    ];
    append_regular_files(&sysroot.join("lib"), &mut files)?;
    append_regular_files(
        &sysroot.join("lib").join("rustlib").join(target).join("lib"),
        &mut files,
    )?;
    files.sort_unstable_by(|left, right| {
        left.as_os_str()
            .as_encoded_bytes()
            .cmp(right.as_os_str().as_encoded_bytes())
    });
    files.dedup();

    let mut digest = Sha256::new();
    for path in &files {
        let relative = path.strip_prefix(sysroot).map_err(|_| {
            format!(
                "U4c toolchain closure path {} escaped sysroot {}",
                path.display(),
                sysroot.display()
            )
        })?;
        let relative = path_to_forward_slashes(relative, "toolchain closure")?;
        digest.update(relative.as_bytes());
        digest.update([0]);
        digest.update(sha256_file(path)?.as_bytes());
        digest.update(b"\n");
    }
    let file_count = u64::try_from(files.len())
        .map_err(|_| "U4c toolchain closure file count overflowed".to_owned())?;
    Ok((file_count, format!("{:x}", digest.finalize())))
}

#[cfg(feature = "repository-input-candidate")]
fn find_unique_libstd(sysroot: &Path, target: &str) -> Result<PathBuf, String> {
    let directory = sysroot.join("lib").join("rustlib").join(target).join("lib");
    let entries = fs::read_dir(&directory).map_err(|error| {
        format!(
            "failed to enumerate U4c target libdir {}: {error}",
            directory.display()
        )
    })?;
    let mut candidates = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|error| {
                format!(
                    "failed to enumerate U4c target libdir {}: {error}",
                    directory.display()
                )
            })?
            .path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("libstd-") && name.ends_with(".rlib"))
        {
            candidates.push(path);
        }
    }
    candidates.sort_unstable();
    match candidates.as_slice() {
        [libstd] if libstd.is_file() => Ok(libstd.clone()),
        _ => Err(format!(
            "U4c production target libdir must contain exactly one libstd rlib, found {}",
            candidates.len()
        )),
    }
}

#[cfg(feature = "repository-input-candidate")]
fn append_regular_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(directory).map_err(|error| {
        format!(
            "failed to enumerate U4c toolchain directory {}: {error}",
            directory.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "failed to enumerate U4c toolchain directory {}: {error}",
                directory.display()
            )
        })?;
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "failed to inspect U4c toolchain entry {}: {error}",
                entry.path().display()
            )
        })?;
        if file_type.is_file() {
            files.push(entry.path());
        } else if file_type.is_symlink() {
            return Err(format!(
                "U4c production toolchain closure rejects symlink {}",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| {
        format!(
            "failed to open U4c evidence file {}: {error}",
            path.display()
        )
    })?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            format!(
                "failed to read U4c evidence file {}: {error}",
                path.display()
            )
        })?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(feature = "repository-input-candidate")]
fn path_to_forward_slashes(path: &Path, label: &str) -> Result<String, String> {
    let mut components = Vec::new();
    for component in path.components() {
        let Component::Normal(component) = component else {
            return Err(format!(
                "U4c production {label} path {} is not relative",
                path.display()
            ));
        };
        let component = component.to_str().ok_or_else(|| {
            format!(
                "U4c production {label} path {} is not valid UTF-8",
                path.display()
            )
        })?;
        components.push(component);
    }
    if components.is_empty() {
        return Err(format!("U4c production {label} path must not be empty"));
    }
    Ok(components.join("/"))
}

#[cfg(feature = "repository-input-candidate")]
enum CargoArtifactTarget<'target> {
    Binary(&'target str),
    LibraryTest(&'target str),
    IntegrationTest(&'target str),
}

#[cfg(feature = "repository-input-candidate")]
impl CargoArtifactTarget<'_> {
    fn cargo_subcommand(&self) -> &str {
        match self {
            Self::Binary(_) => "build",
            Self::LibraryTest(_) | Self::IntegrationTest(_) => "test",
        }
    }

    fn append_cargo_arguments(&self, command: &mut Command) {
        match self {
            Self::Binary(name) => {
                command.args(["--bin", name]);
            }
            Self::LibraryTest(_) => {
                command.arg("--lib");
            }
            Self::IntegrationTest(name) => {
                command.args(["--test", name]);
            }
        }
    }

    fn cargo_target_name(&self) -> &str {
        match self {
            Self::Binary(name) | Self::LibraryTest(name) | Self::IntegrationTest(name) => name,
        }
    }

    const fn is_test_profile(&self) -> bool {
        matches!(self, Self::LibraryTest(_) | Self::IntegrationTest(_))
    }
}

#[cfg(feature = "repository-input-candidate")]
fn prepare_cargo_artifact(
    repository_root: &Path,
    cargo_sandbox: &CargoSandbox,
    dependency_closure: &VerifiedDependencyClosure,
    specification: &ProductionSpecification,
    package: &str,
    features: Option<&str>,
    target: CargoArtifactTarget<'_>,
) -> Result<PreparedCargoArtifact, String> {
    let mut command = cargo_sandbox.build_command()?;
    command
        .arg(target.cargo_subcommand())
        .args([
            "-j",
            "1",
            "--locked",
            "--offline",
            "--target",
            &specification.identity.target,
            "--manifest-path",
        ])
        .arg(repository_root.join("Cargo.toml"))
        .args(["-p", package]);
    if let Some(features) = features {
        command.args(["--features", features]);
    }
    target.append_cargo_arguments(&mut command);
    if target.is_test_profile() {
        command.arg("--no-run");
    }
    command.arg("--message-format=json-render-diagnostics");
    let output = command.output().map_err(|error| {
        format!(
            "failed to prepare U4c Cargo artifact {package}/{}: {error}",
            target.cargo_target_name()
        )
    })?;
    if !output.status.success() {
        return Err(format!(
            "U4c Cargo artifact preparation {package}/{} failed with {}: {}",
            target.cargo_target_name(),
            output.status,
            command_stderr(&output.stderr)
        ));
    }
    let repository_manifest = format!("crates/{package}/Cargo.toml");
    parse_prepared_artifact(
        &output.stdout,
        repository_root,
        &cargo_sandbox.target_dir,
        &dependency_closure.package_by_id,
        &dependency_closure.build_scripts,
        PreparedArtifactExpectation::new(
            package,
            &repository_manifest,
            target.cargo_target_name(),
            target.is_test_profile(),
        ),
    )
}

#[cfg(feature = "repository-input-candidate")]
fn execute_prepared_test(
    cargo_sandbox: &CargoSandbox,
    prepared: &PreparedCargoArtifact,
    working_directory: &Path,
    arguments: &[&str],
    label: &str,
) -> Result<process::Output, String> {
    let output = cargo_sandbox
        .prepared_test_command(prepared, working_directory)?
        .args(arguments)
        .output()
        .map_err(|error| format!("failed to execute {label}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{label} failed with {}: stdout: {}; stderr: {}",
            output.status,
            command_stderr(&output.stdout),
            command_stderr(&output.stderr)
        ));
    }
    Ok(output)
}

#[cfg(feature = "repository-input-candidate")]
fn run_collector_gate(
    repository_root: &Path,
    cargo_sandbox: &CargoSandbox,
    prepared: &PreparedCargoArtifact,
    specification: &ProductionSpecification,
) -> Result<(), String> {
    execute_prepared_test(
        cargo_sandbox,
        prepared,
        &repository_root
            .join("crates")
            .join(&specification.collector_gate.package),
        &["--test-threads=1"],
        "U4c collector gate",
    )?;
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
fn fetch_dependency_sources(
    repository_root: &Path,
    cargo_sandbox: &CargoSandbox,
    specification: &ProductionSpecification,
) -> Result<(), String> {
    let output = cargo_sandbox
        .network_command()
        .args([
            "fetch",
            "--locked",
            "--target",
            &specification.identity.target,
            "--manifest-path",
        ])
        .arg(repository_root.join("Cargo.toml"))
        .output()
        .map_err(|error| format!("failed to fetch admitted U4c dependencies: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "U4c dependency fetch failed with {}: {}",
            output.status,
            command_stderr(&output.stderr)
        ))
    }
}

#[cfg(feature = "repository-input-candidate")]
fn run_production_probe(
    repository_root: &Path,
    cargo_sandbox: &CargoSandbox,
    prepared: &PreparedCargoArtifact,
    specification: &ProductionSpecification,
) -> Result<String, String> {
    let output = execute_prepared_test(
        cargo_sandbox,
        prepared,
        &repository_root
            .join("crates")
            .join(&specification.runner.probe_package),
        &[
            &specification.runner.probe_target,
            "--ignored",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ],
        "U4c production probe",
    )?;
    String::from_utf8(output.stdout)
        .map_err(|error| format!("U4c production probe emitted non-UTF-8 stdout: {error}"))
}

#[cfg(feature = "repository-input-candidate")]
fn verify_dependency_closure(
    repository_root: &Path,
    cargo_sandbox: &CargoSandbox,
    capture: &baozi_io::RepositoryCapture,
    specification: &ProductionSpecification,
) -> Result<VerifiedDependencyClosure, String> {
    let checksums = parse_lock_checksums(captured_file_bytes(capture, "Cargo.lock")?)?;
    let core_metadata = cargo_metadata(
        repository_root,
        cargo_sandbox,
        "crates/baozi-core/Cargo.toml",
        None,
        &specification.identity.target,
    )?;
    let xtask_metadata = cargo_metadata(
        repository_root,
        cargo_sandbox,
        "crates/baozi-xtask/Cargo.toml",
        Some("repository-input-candidate"),
        &specification.identity.target,
    )?;
    let collector_metadata = cargo_metadata(
        repository_root,
        cargo_sandbox,
        "crates/baozi-operation/Cargo.toml",
        Some(specification.collector_gate.feature.as_str()),
        &specification.identity.target,
    )?;

    let mut packages = BTreeSet::new();
    let mut build_scripts = BTreeSet::new();
    let mut proc_macros = BTreeSet::new();
    let mut package_by_id = BTreeMap::new();
    for (metadata, root) in [
        (&core_metadata, PRODUCTION_PROBE_PACKAGE),
        (&xtask_metadata, "baozi-xtask"),
        (
            &collector_metadata,
            specification.collector_gate.package.as_str(),
        ),
    ] {
        verify_workspace_manifest_closure(metadata, repository_root, cargo_sandbox, capture)?;
        let discovered = discover_metadata_closure(
            metadata,
            root,
            &checksums,
            repository_root,
            capture,
            specification,
        )?;
        packages.extend(discovered.packages);
        build_scripts.extend(discovered.build_scripts);
        proc_macros.extend(discovered.proc_macros);
        for (package_id, identity) in discovered.package_by_id {
            if let Some(previous) = package_by_id.insert(package_id.clone(), identity.clone())
                && previous != identity
            {
                return Err(format!(
                    "U4c Cargo metadata assigned conflicting identities to package id {package_id:?}: {previous:?} and {identity:?}"
                ));
            }
        }
    }

    let expected_packages = specification
        .closure
        .packages
        .iter()
        .map(ResolvedPackage::from_specification)
        .collect::<BTreeSet<_>>();
    if packages != expected_packages {
        return Err(format!(
            "U4c production resolved package closure drifted: expected {expected_packages:?}, found {packages:?}"
        ));
    }
    let expected_build_scripts = specification
        .closure
        .build_scripts
        .iter()
        .map(PackageReference::owned)
        .collect::<BTreeSet<_>>();
    if build_scripts != expected_build_scripts {
        return Err(format!(
            "U4c production build-script closure drifted: expected {expected_build_scripts:?}, found {build_scripts:?}"
        ));
    }
    let expected_proc_macros = specification
        .closure
        .proc_macros
        .iter()
        .map(PackageReference::owned)
        .collect::<BTreeSet<_>>();
    if proc_macros != expected_proc_macros {
        return Err(format!(
            "U4c production procedural-macro closure drifted: expected {expected_proc_macros:?}, found {proc_macros:?}"
        ));
    }
    let build_scripts = expected_build_scripts
        .into_iter()
        .map(|package| BuildOutputPackageReference::new(package.name, package.version))
        .collect();
    Ok(VerifiedDependencyClosure {
        package_by_id,
        build_scripts,
    })
}

#[cfg(feature = "repository-input-candidate")]
fn cargo_metadata(
    repository_root: &Path,
    cargo_sandbox: &CargoSandbox,
    manifest_path: &str,
    features: Option<&str>,
    target: &str,
) -> Result<Value, String> {
    let mut command = cargo_sandbox.network_command();
    command.args([
        "metadata",
        "--locked",
        "--format-version",
        "1",
        "--filter-platform",
        target,
        "--manifest-path",
    ]);
    command.arg(repository_root.join(manifest_path));
    if let Some(features) = features {
        command.args(["--features", features]);
    }
    let output = command
        .output()
        .map_err(|error| format!("failed to run Cargo metadata for {manifest_path}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Cargo metadata failed for {manifest_path} with {}: {}",
            output.status,
            command_stderr(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| {
        format!("Cargo metadata emitted invalid JSON for {manifest_path}: {error}")
    })
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ResolvedPackage {
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
    repository_manifest: Option<String>,
}

#[cfg(feature = "repository-input-candidate")]
impl ResolvedPackage {
    fn from_specification(value: &PackageSpecification) -> Self {
        Self {
            name: value.name.clone(),
            version: value.version.clone(),
            source: value.source.clone(),
            checksum: value.checksum.clone(),
            repository_manifest: value.repository_manifest.clone(),
        }
    }
}

#[cfg(feature = "repository-input-candidate")]
impl PackageReference {
    fn owned(&self) -> Self {
        Self {
            name: self.name.clone(),
            version: self.version.clone(),
        }
    }
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug)]
struct MetadataClosure {
    packages: BTreeSet<ResolvedPackage>,
    build_scripts: BTreeSet<PackageReference>,
    proc_macros: BTreeSet<PackageReference>,
    package_by_id: BTreeMap<String, BuildPackageIdentity>,
}

#[cfg(feature = "repository-input-candidate")]
struct VerifiedDependencyClosure {
    package_by_id: BTreeMap<String, BuildPackageIdentity>,
    build_scripts: BTreeSet<BuildOutputPackageReference>,
}

#[cfg(feature = "repository-input-candidate")]
fn discover_metadata_closure(
    metadata: &Value,
    root_package: &str,
    checksums: &BTreeMap<PackageKey, String>,
    repository_root: &Path,
    capture: &baozi_io::RepositoryCapture,
    specification: &ProductionSpecification,
) -> Result<MetadataClosure, String> {
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| "Cargo metadata omitted packages".to_owned())?;
    let nodes = metadata
        .get("resolve")
        .and_then(Value::as_object)
        .and_then(|resolve| resolve.get("nodes"))
        .and_then(Value::as_array)
        .ok_or_else(|| "Cargo metadata omitted resolve nodes".to_owned())?;

    let mut packages_by_id = BTreeMap::new();
    for package in packages {
        let id = json_string(package, "id", "Cargo metadata package")?;
        if packages_by_id.insert(id, package).is_some() {
            return Err(format!("Cargo metadata duplicated package id {id}"));
        }
    }
    let root_ids = packages
        .iter()
        .filter(|package| package.get("name").and_then(Value::as_str) == Some(root_package))
        .filter_map(|package| package.get("id").and_then(Value::as_str))
        .collect::<Vec<_>>();
    let root_id = match root_ids.as_slice() {
        [root_id] => *root_id,
        _ => {
            return Err(format!(
                "Cargo metadata must expose exactly one root package {root_package:?}, found {}",
                root_ids.len()
            ));
        }
    };
    let mut nodes_by_id = BTreeMap::new();
    for node in nodes {
        let id = json_string(node, "id", "Cargo metadata resolve node")?;
        if nodes_by_id.insert(id, node).is_some() {
            return Err(format!("Cargo metadata duplicated resolve node {id}"));
        }
    }

    let mut pending = VecDeque::from([root_id]);
    let mut visited = BTreeSet::new();
    let mut discovered = MetadataClosure {
        packages: BTreeSet::new(),
        build_scripts: BTreeSet::new(),
        proc_macros: BTreeSet::new(),
        package_by_id: BTreeMap::new(),
    };
    while let Some(id) = pending.pop_front() {
        if !visited.insert(id) {
            continue;
        }
        let package = packages_by_id
            .get(id)
            .copied()
            .ok_or_else(|| format!("Cargo metadata resolve references unknown package {id}"))?;
        let name = json_string(package, "name", "Cargo metadata package")?;
        let version = json_string(package, "version", "Cargo metadata package")?;
        let source = package
            .get("source")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let repository_manifest = if source.is_none() {
            Some(verify_local_package_capture(
                package,
                repository_root,
                capture,
                specification,
                name,
                version,
            )?)
        } else {
            None
        };
        let checksum = match &source {
            Some(source) => checksums
                .get(&PackageKey {
                    name: name.to_owned(),
                    version: version.to_owned(),
                    source: source.clone(),
                })
                .cloned()
                .ok_or_else(|| {
                    format!("Cargo.lock omitted checksum for {name}@{version} from {source}")
                })?,
            None => String::new(),
        };
        discovered.packages.insert(ResolvedPackage {
            name: name.to_owned(),
            version: version.to_owned(),
            source: source.clone(),
            checksum: if checksum.is_empty() {
                None
            } else {
                Some(checksum)
            },
            repository_manifest: repository_manifest.clone(),
        });
        let identity = BuildPackageIdentity::new(
            name.to_owned(),
            version.to_owned(),
            source,
            repository_manifest,
        );
        if let Some(previous) = discovered
            .package_by_id
            .insert(id.to_owned(), identity.clone())
            && previous != identity
        {
            return Err(format!(
                "Cargo metadata assigned conflicting identities to package id {id}: {previous:?} and {identity:?}"
            ));
        }

        let reference = PackageReference {
            name: name.to_owned(),
            version: version.to_owned(),
        };
        let targets = package
            .get("targets")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("Cargo metadata package {name}@{version} omitted targets"))?;
        for target in targets {
            let kinds = target
                .get("kind")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    format!("Cargo metadata target for {name}@{version} omitted kind")
                })?;
            let kinds = kinds
                .iter()
                .map(|kind| {
                    kind.as_str().ok_or_else(|| {
                        format!("Cargo metadata target kind for {name}@{version} was not text")
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            if kinds.contains(&"custom-build") {
                discovered.build_scripts.insert(reference.owned());
            }
            if kinds.contains(&"proc-macro") {
                discovered.proc_macros.insert(reference.owned());
            }
        }

        let node = nodes_by_id
            .get(id)
            .copied()
            .ok_or_else(|| format!("Cargo metadata omitted resolve node for {id}"))?;
        let dependencies = node
            .get("deps")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("Cargo metadata resolve node {id} omitted dependencies"))?;
        for dependency in dependencies {
            pending.push_back(json_string(dependency, "pkg", "Cargo metadata dependency")?);
        }
    }
    Ok(discovered)
}

#[cfg(feature = "repository-input-candidate")]
fn verify_workspace_manifest_closure(
    metadata: &Value,
    repository_root: &Path,
    cargo_sandbox: &CargoSandbox,
    capture: &baozi_io::RepositoryCapture,
) -> Result<(), String> {
    let workspace_root = metadata
        .get("workspace_root")
        .and_then(Value::as_str)
        .ok_or_else(|| "Cargo metadata omitted workspace_root".to_owned())?;
    if Path::new(workspace_root) != repository_root {
        return Err(format!(
            "U4c production Cargo workspace root drifted: expected {}, found {workspace_root}",
            repository_root.display()
        ));
    }
    let target_directory = metadata
        .get("target_directory")
        .and_then(Value::as_str)
        .ok_or_else(|| "Cargo metadata omitted target_directory".to_owned())?;
    if Path::new(target_directory) != cargo_sandbox.target_dir {
        return Err(format!(
            "U4c production Cargo target directory escaped its sandbox: expected {}, found {target_directory}",
            cargo_sandbox.target_dir.display()
        ));
    }

    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| "Cargo metadata omitted packages".to_owned())?;
    let mut packages_by_id = BTreeMap::new();
    for package in packages {
        let id = json_string(package, "id", "Cargo metadata package")?;
        if packages_by_id.insert(id, package).is_some() {
            return Err(format!("Cargo metadata duplicated package id {id}"));
        }
    }
    let workspace_members = metadata
        .get("workspace_members")
        .and_then(Value::as_array)
        .ok_or_else(|| "Cargo metadata omitted workspace_members".to_owned())?;
    let mut actual = BTreeSet::new();
    for member in workspace_members {
        let id = member
            .as_str()
            .ok_or_else(|| "Cargo metadata workspace member id was not text".to_owned())?;
        let package = packages_by_id
            .get(id)
            .copied()
            .ok_or_else(|| format!("Cargo metadata workspace references unknown package {id}"))?;
        let manifest_path = metadata_repository_path(
            repository_root,
            json_string(package, "manifest_path", "Cargo metadata workspace package")?,
            "workspace manifest",
        )?;
        captured_file_bytes(capture, &manifest_path)?;
        actual.insert(manifest_path);
    }
    let expected = capture
        .files()
        .iter()
        .map(|file| file.path().as_str())
        .filter(|path| {
            let components = path.split('/').collect::<Vec<_>>();
            matches!(components.as_slice(), ["crates", _, "Cargo.toml"])
        })
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(format!(
            "U4c production workspace manifest closure drifted: expected {expected:?}, found {actual:?}"
        ));
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
fn verify_local_package_capture(
    package: &Value,
    repository_root: &Path,
    capture: &baozi_io::RepositoryCapture,
    specification: &ProductionSpecification,
    name: &str,
    version: &str,
) -> Result<String, String> {
    let manifest_path = metadata_repository_path(
        repository_root,
        json_string(package, "manifest_path", "Cargo metadata local package")?,
        "local package manifest",
    )?;
    let source_tree = specification
        .closure
        .source_trees
        .iter()
        .find(|tree| repository_path_is_prefix(&tree.path, &manifest_path))
        .ok_or_else(|| {
            format!(
                "U4c production local package {name}@{version} manifest {manifest_path} is outside the captured source trees"
            )
        })?;
    captured_file_bytes(capture, &manifest_path)?;

    let targets = package
        .get("targets")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("Cargo metadata package {name}@{version} omitted targets"))?;
    for target in targets {
        let source_path = metadata_repository_path(
            repository_root,
            json_string(target, "src_path", "Cargo metadata local target")?,
            "local target source",
        )?;
        if !repository_path_is_prefix(&source_tree.path, &source_path) {
            return Err(format!(
                "U4c production local target {source_path} for {name}@{version} escapes captured source tree {}",
                source_tree.path
            ));
        }
        captured_file_bytes(capture, &source_path)?;
    }
    Ok(manifest_path)
}

#[cfg(feature = "repository-input-candidate")]
fn metadata_repository_path(
    repository_root: &Path,
    path: &str,
    label: &str,
) -> Result<String, String> {
    let relative = Path::new(path).strip_prefix(repository_root).map_err(|_| {
        format!(
            "U4c production {label} path {path:?} is outside repository root {}",
            repository_root.display()
        )
    })?;
    let mut components = Vec::new();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(format!(
                "U4c production {label} path {path:?} is not repository-relative"
            ));
        };
        let component = component
            .to_str()
            .ok_or_else(|| format!("U4c production {label} path {path:?} is not valid UTF-8"))?;
        if !component.is_ascii() {
            return Err(format!("U4c production {label} path {path:?} is not ASCII"));
        }
        components.push(component);
    }
    if components.is_empty() {
        return Err(format!(
            "U4c production {label} path {path:?} names the repository root"
        ));
    }
    Ok(components.join("/"))
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PackageKey {
    name: String,
    version: String,
    source: String,
}

#[cfg(feature = "repository-input-candidate")]
fn parse_lock_checksums(bytes: &[u8]) -> Result<BTreeMap<PackageKey, String>, String> {
    let contents =
        std::str::from_utf8(bytes).map_err(|error| format!("Cargo.lock must be UTF-8: {error}"))?;
    let lock = toml::from_str::<toml::Value>(contents)
        .map_err(|error| format!("failed to parse captured Cargo.lock: {error}"))?;
    let packages = lock
        .get("package")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "captured Cargo.lock omitted package array".to_owned())?;
    let mut checksums = BTreeMap::new();
    for package in packages {
        let table = package
            .as_table()
            .ok_or_else(|| "captured Cargo.lock package was not a table".to_owned())?;
        let Some(source) = table.get("source").and_then(toml::Value::as_str) else {
            continue;
        };
        let name = table
            .get("name")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| "captured Cargo.lock package omitted name".to_owned())?;
        let version = table
            .get("version")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| format!("captured Cargo.lock package {name} omitted version"))?;
        let checksum = table
            .get("checksum")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| {
                format!("captured Cargo.lock package {name}@{version} omitted checksum")
            })?;
        if !is_lowercase_sha256(checksum) {
            return Err(format!(
                "captured Cargo.lock package {name}@{version} has invalid checksum"
            ));
        }
        let key = PackageKey {
            name: name.to_owned(),
            version: version.to_owned(),
            source: source.to_owned(),
        };
        if checksums.insert(key, checksum.to_owned()).is_some() {
            return Err(format!(
                "captured Cargo.lock duplicated checksum identity {name}@{version}"
            ));
        }
    }
    Ok(checksums)
}

#[cfg(feature = "repository-input-candidate")]
fn json_string<'a>(value: &'a Value, key: &str, label: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{label} omitted string field {key:?}"))
}

#[cfg(feature = "repository-input-candidate")]
fn metadata_line<'a>(output: &'a str, key: &str) -> Result<&'a str, String> {
    let prefix = format!("{key}: ");
    let mut values = output.lines().filter_map(|line| line.strip_prefix(&prefix));
    let value = values
        .next()
        .ok_or_else(|| format!("Cargo-selected rustc did not emit {key:?}"))?;
    if values.next().is_some() {
        return Err(format!(
            "Cargo-selected rustc emitted duplicate {key:?} metadata"
        ));
    }
    Ok(value)
}

#[cfg(feature = "repository-input-candidate")]
fn command_stderr(stderr: &[u8]) -> String {
    let diagnostic = String::from_utf8_lossy(stderr);
    let diagnostic = diagnostic.trim();
    if diagnostic.is_empty() {
        "no stderr was emitted".to_owned()
    } else {
        diagnostic.to_owned()
    }
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProductionReport {
    schema_id: String,
    schema_version: u64,
    report_revision: u64,
    scenario_set_id: String,
    scale_scenarios: Vec<ScaleScenarioReport>,
    inherited_hot_leaf_scenario: InheritedHotLeafReport,
    obj_capability: obj::ObjCapabilityReport,
    gltf_capability: gltf::GltfCapabilityReport,
    profile_closure: profile::ProfileClosureReport,
    validation_faults: Vec<ValidationFaultReport>,
    pending_scene_activation: PendingSceneActivationReport,
    protected_terminal_activation: ProtectedTerminalActivationReport,
    minimal_fallback: MinimalFallbackReport,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScaleScenarioReport {
    scenario_id: String,
    image_count: u64,
    canonical_block_count: u64,
    foreign_canonical_block_count: u64,
    target_generation_reservations: u64,
    foreign_generation_reservations: u64,
    typed_inspection_succeeded: bool,
    foreign_directory_rejected: bool,
    semantic_transcript: SemanticTranscriptReport,
    evidence: EvidenceReport,
    construction: PhaseReport,
    typed_inspection: PhaseReport,
    target_release: ReleaseReport,
    foreign_release: ReleaseReport,
    physical_account: ScalePhysicalReport,
    factory_after_account_release: PhysicalFactoryReport,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InheritedHotLeafReport {
    scenario_id: String,
    generation_reservations: u64,
    fresh: ShareGenerationReport,
    derived: ShareGenerationReport,
    fresh_hot_leaf: HotLeafObservationReport,
    derived_hot_leaf: HotLeafObservationReport,
    derived_after_parent_release_hot_leaf: HotLeafObservationReport,
    fresh_report_release: ReleaseReport,
    derived_report_release: ReleaseReport,
    parent_release: ReleaseReport,
    derived_release: ReleaseReport,
    physical_account: SharePhysicalReport,
    factory_after_account_release: PhysicalFactoryReport,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ValidationFaultReport {
    scenario_id: String,
    field_class: String,
    mutation_schema_id: String,
    mutation_transcript_bytes: u64,
    mutation_transcript_sha256: String,
    image_count: u64,
    generation_reservations: u64,
    evidence: EvidenceReport,
    construction: PhaseReport,
    release: ReleaseReport,
    physical_account: FaultPhysicalReport,
    factory_after_account_release: PhysicalFactoryReport,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MinimalFallbackReport {
    scenario_id: String,
    fallback_code: u32,
    fallback_output_count: u64,
    evidence: EvidenceReport,
    construction: PhaseReport,
    handoff: HandoffReport,
    release: ReleaseReport,
    physical_account: FaultPhysicalReport,
    factory_after_account_release: PhysicalFactoryReport,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProtectedTerminalActivationReport {
    scenario_id: String,
    evidence: EvidenceReport,
    construction: PhaseReport,
    handoff: HandoffReport,
    release: ReleaseReport,
    physical_account: FaultPhysicalReport,
    factory_after_account_release: PhysicalFactoryReport,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingSceneActivationReport {
    entry_point: String,
    scenario_id: String,
    canonical_block_count: u64,
    outcome_evidence_block_count: u64,
    new_block_count: u64,
    directory_identity_preserved: bool,
    construction: PhaseReport,
    candidate_release: ReleaseReport,
    physical_account: PendingSceneActivationPhysicalReport,
    factory_after_account_release: PhysicalFactoryReport,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShareGenerationReport {
    construction: PhaseReport,
    evidence: EvidenceReport,
    canonical_block_count: u64,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HotLeafObservationReport {
    phase: PhaseReport,
    matrix_bits: Vec<u32>,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SemanticTranscriptReport {
    schema_id: String,
    observed_bytes: u64,
    sha256: String,
    bounded_preview: String,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceReport {
    retained_evidence_block_count: u64,
    ledger_header: LedgerHeaderReport,
    dimensions: Vec<LedgerDimensionReport>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum LedgerDimensionRole {
    Ordinary,
    FailureTerminal,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
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

impl LedgerDimensionReport {
    fn has_terminal_invariants(&self) -> bool {
        self.semantic_id_hex.len() == 58
            && self
                .semantic_id_hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            && self.attempted >= self.consumed
            && self.current_live_bytes == 0
            && self.peak_live_bytes >= self.current_live_bytes
            && self.pending_publication == 0
            && self.committed_canonical_output <= self.committed
    }

    fn has_zero_protected_storage_and_publication(&self) -> bool {
        self.current_live_bytes == 0
            && self.peak_live_bytes == 0
            && self.pending_publication == 0
            && self.committed == 0
            && self.committed_canonical_output == 0
            && self.published_outcome_evidence == 0
    }

    fn is_unused_failure_terminal(&self) -> bool {
        self.attempted == 0
            && self.consumed == 0
            && self.has_zero_protected_storage_and_publication()
    }

    fn matches_protected_terminal(&self, attempted: u64, consumed: u64) -> bool {
        self.attempted == attempted
            && self.consumed == consumed
            && self.has_zero_protected_storage_and_publication()
    }
}

struct LedgerDimensionsByRole<'report> {
    ordinary: &'report LedgerDimensionReport,
    failure_terminal: &'report LedgerDimensionReport,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LedgerHeaderReport {
    schema_revision: u64,
    terminal_cause: TerminalCauseReport,
    dimension_count: u64,
    phase_count: u64,
    current_live_bytes: u64,
    peak_live_bytes: u64,
    active_reservations: u64,
    current_evidence_bytes: u64,
    peak_evidence_bytes: u64,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TerminalCauseReport {
    class: String,
    failure_class: Option<String>,
    code: Option<u64>,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PhaseReport {
    scenario_id: String,
    taxonomy: MeasurementTaxonomyReport,
    allocations: AllocationReport,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseReport {
    scenario_id: String,
    allocations: AllocationReport,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HandoffReport {
    scenario_id: String,
    allocations: AllocationReport,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MeasurementTaxonomyReport {
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

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct AllocationReport {
    count_total: u64,
    count_current: i64,
    count_max: u64,
    bytes_total: u64,
    bytes_current: i64,
    bytes_max: u64,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScalePhysicalReport {
    foreign_baseline: PhysicalAccountReport,
    target_retained: PhysicalAccountReport,
    after_typed_inspection: PhysicalAccountReport,
    after_target_release: PhysicalAccountReport,
    after_foreign_release: PhysicalAccountReport,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SharePhysicalReport {
    baseline: PhysicalAccountReport,
    fresh_handoff: PhysicalAccountReport,
    fresh_scene_only: PhysicalAccountReport,
    derived_handoff: PhysicalAccountReport,
    parent_and_derived_scenes: PhysicalAccountReport,
    after_parent_release: PhysicalAccountReport,
    after_derived_release: PhysicalAccountReport,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FaultPhysicalReport {
    baseline: PhysicalAccountReport,
    retained: PhysicalAccountReport,
    after_handoff: PhysicalAccountReport,
    after_release: PhysicalAccountReport,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingSceneActivationPhysicalReport {
    baseline: PhysicalAccountReport,
    candidate_retained: PhysicalAccountReport,
    after_candidate_release: PhysicalAccountReport,
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct PhysicalAccountReport {
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

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PhysicalFactoryReport {
    live_account_control_bytes: u64,
    live_accounts: u64,
    peak_account_control_bytes: u64,
    peak_live_accounts: u64,
    total_admitted_account_control_bytes: u64,
    total_admitted_accounts: u64,
}

#[cfg(feature = "repository-input-candidate")]
fn parse_production_probe_stdout(
    stdout: &str,
    prefix: &str,
) -> Result<(String, ProductionReport), String> {
    let mut payload = None;
    let mut prefix_matches = 0_u64;
    for line in stdout.lines() {
        if let Some(candidate) = line.strip_prefix(prefix) {
            prefix_matches = prefix_matches
                .checked_add(1)
                .ok_or_else(|| "U4c production report prefix count overflowed".to_owned())?;
            if prefix_matches == 1 {
                payload = Some(candidate);
            }
        }
    }
    let payload = match prefix_matches {
        0 => return Err("U4c production probe emitted no report prefix".to_owned()),
        1 => payload.ok_or_else(|| "U4c production probe omitted report payload".to_owned())?,
        _ => {
            return Err(format!(
                "U4c production probe emitted {prefix_matches} report prefixes"
            ));
        }
    };
    let report = serde_json::from_str::<ProductionReport>(payload)
        .map_err(|error| format!("U4c production probe emitted invalid report JSON: {error}"))?;
    Ok((payload.to_owned(), report))
}

#[cfg(feature = "repository-input-candidate")]
fn validate_production_report(
    specification: &ProductionSpecification,
    payload: &str,
    report: &ProductionReport,
) -> Result<ProductionEvidenceSummary, String> {
    let contract = &specification.report_contract;
    if report.schema_id != contract.schema_id
        || report.schema_version != contract.schema_version
        || report.report_revision != contract.report_revision
        || report.scenario_set_id != contract.scenario_set_id
    {
        return Err("U4c production report header drifted from the frozen contract".to_owned());
    }
    let payload_bytes = u64::try_from(payload.len())
        .map_err(|_| "U4c production report payload length overflowed".to_owned())?;
    if payload_bytes != contract.payload_bytes {
        return Err(format!(
            "U4c production report byte length drifted: expected {}, found {payload_bytes}",
            contract.payload_bytes
        ));
    }
    let payload_hash = sha256_hex(payload.as_bytes());
    if payload_hash != contract.sha256 {
        return Err(format!(
            "U4c production report digest drifted: expected {}, found {payload_hash}",
            contract.sha256
        ));
    }

    validate_scale_reports(specification, &report.scale_scenarios)?;
    validate_inherited_hot_leaf(specification, &report.inherited_hot_leaf_scenario)?;
    obj::validate_report(specification, &report.obj_capability)?;
    gltf::validate_report(specification, &report.gltf_capability)?;
    profile::validate_report(specification, &report.profile_closure)?;
    validate_validation_faults(specification, &report.validation_faults)?;
    validate_pending_scene_activation(specification, &report.pending_scene_activation)?;
    validate_protected_terminal_activation(specification, &report.protected_terminal_activation)?;
    validate_minimal_fallback(specification, &report.minimal_fallback)?;

    Ok(ProductionEvidenceSummary::after_validated_private_tracer_report())
}

#[cfg(feature = "repository-input-candidate")]
fn validate_scale_reports(
    specification: &ProductionSpecification,
    reports: &[ScaleScenarioReport],
) -> Result<(), String> {
    let report_ids = reports
        .iter()
        .map(|report| report.scenario_id.as_str())
        .collect::<Vec<_>>();
    validate_exact_list(
        "observed scale scenarios",
        &report_ids,
        REQUIRED_SCALE_SCENARIOS,
    )?;
    for expected in &specification.scenarios {
        let report = reports
            .iter()
            .find(|report| report.scenario_id == expected.scenario_id)
            .ok_or_else(|| {
                format!(
                    "U4c production omitted scale scenario {}",
                    expected.scenario_id
                )
            })?;
        let vector = specification
            .semantic_vectors
            .iter()
            .find(|vector| vector.scenario_id == expected.scenario_id)
            .ok_or_else(|| {
                format!(
                    "U4c production omitted semantic vector for {}",
                    expected.scenario_id
                )
            })?;
        if report.image_count != expected.image_count
            || report.canonical_block_count != expected.canonical_block_count
            || report.foreign_canonical_block_count == 0
            || report.target_generation_reservations != 1
            || report.foreign_generation_reservations != 1
            || !report.typed_inspection_succeeded
            || !report.foreign_directory_rejected
        {
            return Err(format!(
                "U4c production scale scenario {} structural evidence drifted",
                expected.scenario_id
            ));
        }
        if report.semantic_transcript.schema_id != "baozi.complex-fixture.semantic-transcript.v1"
            || report.semantic_transcript.observed_bytes != vector.observed_bytes
            || report.semantic_transcript.sha256 != vector.sha256
            || !is_lowercase_sha256(&report.semantic_transcript.sha256)
            || !report.semantic_transcript.bounded_preview.is_ascii()
            || report.semantic_transcript.bounded_preview.len() > 192
        {
            return Err(format!(
                "U4c production semantic vector {} drifted",
                expected.scenario_id
            ));
        }
        validate_success_evidence(
            &specification.ledger_contract,
            &report.evidence,
            expected.canonical_block_count,
            &expected.scenario_id,
        )?;
        validate_expected_producer_phase(
            &report.construction,
            expected,
            &format!("{} construction", expected.scenario_id),
        )?;
        validate_hot_view_phase(
            &report.typed_inspection,
            &specification.measurement,
            &format!("{} typed inspection", expected.scenario_id),
        )?;
        validate_scale_lookup_phase(
            &report.typed_inspection,
            &specification.measurement,
            &format!("{} typed inspection", expected.scenario_id),
        )?;
        validate_release(
            &report.target_release,
            &format!("{} target release", expected.scenario_id),
        )?;
        validate_release(
            &report.foreign_release,
            &format!("{} foreign release", expected.scenario_id),
        )?;
        validate_healthy_account(
            &report.physical_account.foreign_baseline,
            &format!("{} foreign baseline", expected.scenario_id),
        )?;
        validate_healthy_account(
            &report.physical_account.target_retained,
            &format!("{} target retained", expected.scenario_id),
        )?;
        let target_live_retained = live_retained_charge(&report.physical_account.target_retained);
        let expected_live_retained = (
            expected.target_live_retained_bytes,
            expected.target_live_retained_allocations,
        );
        if target_live_retained != expected_live_retained {
            return Err(format!(
                "U4c production scale scenario {} physical target charge drifted: expected {:?} bytes/allocations, found {:?}",
                expected.scenario_id, expected_live_retained, target_live_retained,
            ));
        }
        validate_healthy_account(
            &report.physical_account.after_typed_inspection,
            &format!("{} after typed inspection", expected.scenario_id),
        )?;
        validate_healthy_account(
            &report.physical_account.after_target_release,
            &format!("{} after target release", expected.scenario_id),
        )?;
        if live_retained_charge(&report.physical_account.after_typed_inspection)
            != live_retained_charge(&report.physical_account.target_retained)
            || live_retained_charge(&report.physical_account.after_target_release)
                != live_retained_charge(&report.physical_account.foreign_baseline)
        {
            return Err(format!(
                "U4c production scale scenario {} physical live-charge lifecycle drifted",
                expected.scenario_id
            ));
        }
        validate_released_account(
            &report.physical_account.after_foreign_release,
            &format!("{} after foreign release", expected.scenario_id),
        )?;
        validate_released_factory(
            &report.factory_after_account_release,
            &format!("{} factory release", expected.scenario_id),
        )?;
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
fn validate_inherited_hot_leaf(
    specification: &ProductionSpecification,
    report: &InheritedHotLeafReport,
) -> Result<(), String> {
    if report.scenario_id != "fresh-derived-exact-input-inherited-hot-leaf-v1"
        || report.generation_reservations != 2
        || report.fresh.canonical_block_count == 0
        || report.derived.canonical_block_count != report.fresh.canonical_block_count
    {
        return Err("U4c production inherited hot-leaf structure drifted".to_owned());
    }
    validate_producer_bound(
        &report.fresh.construction,
        "U4c production fresh inherited construction",
    )?;
    validate_producer_bound(
        &report.derived.construction,
        "U4c production derived inherited construction",
    )?;
    validate_inherited_share_phase(
        &report.derived.construction,
        "U4c production derived inherited construction",
    )?;
    validate_success_evidence(
        &specification.ledger_contract,
        &report.fresh.evidence,
        report.fresh.canonical_block_count,
        "U4c production fresh inherited",
    )?;
    validate_success_evidence(
        &specification.ledger_contract,
        &report.derived.evidence,
        report.derived.canonical_block_count,
        "U4c production derived inherited",
    )?;
    let hot_views = [
        &report.fresh_hot_leaf,
        &report.derived_hot_leaf,
        &report.derived_after_parent_release_hot_leaf,
    ];
    let expected_matrix = &report.fresh_hot_leaf.matrix_bits;
    if expected_matrix.len() != 16
        || hot_views
            .iter()
            .any(|view| view.matrix_bits.as_slice() != expected_matrix.as_slice())
    {
        return Err("U4c production inherited hot-leaf matrix evidence drifted".to_owned());
    }
    for hot_view in hot_views {
        validate_hot_view_phase(
            &hot_view.phase,
            &specification.measurement,
            "U4c production inherited hot leaf",
        )?;
    }
    for release in [
        &report.fresh_report_release,
        &report.derived_report_release,
        &report.parent_release,
        &report.derived_release,
    ] {
        validate_release(release, "U4c production inherited release")?;
    }
    for snapshot in [
        &report.physical_account.baseline,
        &report.physical_account.fresh_handoff,
        &report.physical_account.fresh_scene_only,
        &report.physical_account.derived_handoff,
        &report.physical_account.parent_and_derived_scenes,
        &report.physical_account.after_parent_release,
    ] {
        validate_healthy_account(snapshot, "U4c production inherited physical account")?;
    }
    validate_released_account(
        &report.physical_account.after_derived_release,
        "U4c production inherited final release",
    )?;
    validate_released_factory(
        &report.factory_after_account_release,
        "U4c production inherited factory release",
    )
}

#[cfg(feature = "repository-input-candidate")]
fn validate_validation_faults(
    specification: &ProductionSpecification,
    reports: &[ValidationFaultReport],
) -> Result<(), String> {
    let report_ids = reports
        .iter()
        .map(|report| report.scenario_id.as_str())
        .collect::<Vec<_>>();
    validate_exact_list(
        "observed mutation scenarios",
        &report_ids,
        REQUIRED_MUTATION_SCENARIOS,
    )?;
    for expected in &specification.mutations {
        let report = reports
            .iter()
            .find(|report| report.scenario_id == expected.scenario_id)
            .ok_or_else(|| format!("U4c production omitted mutation {}", expected.scenario_id))?;
        if report.field_class != expected.field_class
            || report.mutation_schema_id != "baozi.complex-fixture.mutation-transcript.v1"
            || report.mutation_transcript_bytes != expected.transcript_bytes
            || report.mutation_transcript_sha256 != expected.transcript_sha256
            || report.image_count == 0
            || report.generation_reservations != 0
            || report.evidence.retained_evidence_block_count != expected.retained_evidence_blocks
            || report.evidence.ledger_header.terminal_cause.class != "failure"
            || report
                .evidence
                .ledger_header
                .terminal_cause
                .failure_class
                .as_deref()
                != Some("requested")
            || report.evidence.ledger_header.terminal_cause.code != Some(expected.failure_code)
        {
            return Err(format!(
                "U4c production mutation {} semantic evidence drifted",
                expected.scenario_id
            ));
        }
        validate_rich_failure_evidence(
            &specification.ledger_contract,
            &report.evidence,
            0,
            0,
            &expected.scenario_id,
        )?;
        validate_producer_bound(
            &report.construction,
            &format!("U4c production mutation {}", expected.scenario_id),
        )?;
        validate_release(
            &report.release,
            &format!("U4c production mutation {} release", expected.scenario_id),
        )?;
        validate_healthy_account(
            &report.physical_account.baseline,
            &format!("U4c production mutation {} baseline", expected.scenario_id),
        )?;
        validate_healthy_account(
            &report.physical_account.retained,
            &format!("U4c production mutation {} retained", expected.scenario_id),
        )?;
        validate_healthy_account(
            &report.physical_account.after_handoff,
            &format!("U4c production mutation {} handoff", expected.scenario_id),
        )?;
        validate_released_account(
            &report.physical_account.after_release,
            &format!("U4c production mutation {} release", expected.scenario_id),
        )?;
        validate_released_factory(
            &report.factory_after_account_release,
            &format!(
                "U4c production mutation {} factory release",
                expected.scenario_id
            ),
        )?;
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
fn validate_pending_scene_activation(
    specification: &ProductionSpecification,
    report: &PendingSceneActivationReport,
) -> Result<(), String> {
    let expected = &specification.pending_scene_activation;
    if report.entry_point != expected.entry_point
        || report.scenario_id != expected.scenario_id
        || report.canonical_block_count != expected.canonical_block_count
        || report.outcome_evidence_block_count != expected.outcome_evidence_block_count
        || report.new_block_count != expected.new_block_count
        || !report.directory_identity_preserved
    {
        return Err("U4d pending scene activation semantic evidence drifted".to_owned());
    }

    let taxonomy = &report.construction.taxonomy;
    if taxonomy.semantic_cell_writes != expected.semantic_cells
        || taxonomy.reference_writes != expected.references
        || taxonomy.producer_events != expected.producer_events
        || taxonomy.payload_copy_bytes == 0
        || taxonomy.payload_copy_bytes > expected.max_payload_copy_bytes
        || taxonomy.payload_backing_allocations != expected.new_block_count
        || taxonomy.block_begins != expected.new_block_count
        || taxonomy.block_seals != expected.new_block_count
        || taxonomy.slot_publications != expected.new_block_count
        || taxonomy.new_block_begins != expected.new_block_count
        || taxonomy.new_block_seals != expected.new_block_count
        || taxonomy.new_slot_publications != expected.new_block_count
        || taxonomy.inherited_block_begins != 0
        || taxonomy.inherited_block_seals != 0
        || taxonomy.inherited_slot_publications != 0
        || taxonomy.element_decodes != 0
        || taxonomy.completed_lookups != 0
        || taxonomy.rejected_lookups != 0
        || taxonomy.lookup_events != 0
        || taxonomy.max_lookup_events != 0
        || taxonomy.member_probes != 0
        || taxonomy.max_member_probes_per_lookup != 0
    {
        return Err("U4d pending scene activation construction transcript drifted".to_owned());
    }
    validate_producer_bound(
        &report.construction,
        "U4d pending scene activation construction",
    )?;
    validate_release(
        &report.candidate_release,
        "U4d pending scene activation candidate release",
    )?;

    let physical = &report.physical_account;
    validate_released_account(&physical.baseline, "U4d pending scene activation baseline")?;
    validate_healthy_account(
        &physical.candidate_retained,
        "U4d pending scene activation candidate",
    )?;
    if physical.baseline.peak_retained_bytes != 0
        || physical.baseline.peak_retained_allocations != 0
        || physical.baseline.total_admitted_retained_bytes != 0
        || physical.baseline.total_admitted_retained_allocations != 0
        || physical.candidate_retained.live_retained_bytes == 0
        || physical.candidate_retained.live_retained_allocations == 0
    {
        return Err(
            "U4d pending scene activation did not retain a fresh physical candidate".to_owned(),
        );
    }
    validate_released_account(
        &physical.after_candidate_release,
        "U4d pending scene activation candidate release",
    )?;
    if physical.after_candidate_release.peak_retained_bytes
        != physical.candidate_retained.peak_retained_bytes
        || physical.after_candidate_release.peak_retained_allocations
            != physical.candidate_retained.peak_retained_allocations
        || physical
            .after_candidate_release
            .total_admitted_retained_bytes
            != physical.candidate_retained.total_admitted_retained_bytes
        || physical
            .after_candidate_release
            .total_admitted_retained_allocations
            != physical
                .candidate_retained
                .total_admitted_retained_allocations
    {
        return Err(
            "U4d pending scene activation physical history drifted during candidate release"
                .to_owned(),
        );
    }
    validate_released_factory(
        &report.factory_after_account_release,
        "U4d pending scene activation factory release",
    )
}

#[cfg(feature = "repository-input-candidate")]
fn validate_minimal_fallback(
    specification: &ProductionSpecification,
    report: &MinimalFallbackReport,
) -> Result<(), String> {
    let expected = &specification.minimal_fallback;
    let terminal = &report.evidence.ledger_header.terminal_cause;
    if report.scenario_id != expected.scenario_id
        || report.fallback_code != expected.fallback_code
        || report.fallback_output_count != expected.fallback_output_count
        || report.evidence.retained_evidence_block_count != 0
        || terminal.class != "failure"
        || terminal.failure_class.as_deref() != Some(expected.terminal_failure_class.as_str())
        || terminal.code != Some(u64::from(expected.terminal_failure_code))
    {
        return Err("U4c production minimal-fallback semantic evidence drifted".to_owned());
    }
    validate_minimal_fallback_evidence(
        &specification.ledger_contract,
        &report.evidence,
        expected.fallback_output_count,
        &expected.scenario_id,
    )?;
    validate_producer_bound(
        &report.construction,
        "U4c production minimal-fallback construction",
    )?;
    validate_zero_allocation_handoff(&report.handoff, "U4c production minimal-fallback handoff")?;
    validate_physical_handoff_preserved(
        &report.physical_account,
        "U4c production minimal-fallback handoff",
    )?;
    validate_release(&report.release, "U4c production minimal-fallback release")?;
    validate_healthy_account(
        &report.physical_account.baseline,
        "U4c production minimal-fallback baseline",
    )?;
    validate_healthy_account(
        &report.physical_account.retained,
        "U4c production minimal-fallback retained",
    )?;
    validate_healthy_account(
        &report.physical_account.after_handoff,
        "U4c production minimal-fallback handoff",
    )?;
    validate_released_account(
        &report.physical_account.after_release,
        "U4c production minimal-fallback release",
    )?;
    validate_released_factory(
        &report.factory_after_account_release,
        "U4c production minimal-fallback factory release",
    )
}

#[cfg(feature = "repository-input-candidate")]
fn validate_protected_terminal_activation(
    specification: &ProductionSpecification,
    report: &ProtectedTerminalActivationReport,
) -> Result<(), String> {
    let expected = &specification.protected_terminal_activation;
    let terminal = &report.evidence.ledger_header.terminal_cause;
    if report.scenario_id != expected.scenario_id
        || report.evidence.retained_evidence_block_count != expected.retained_evidence_blocks
        || terminal.class != "failure"
        || terminal.failure_class.as_deref() != Some(expected.terminal_failure_class.as_str())
        || terminal.code != Some(u64::from(expected.terminal_failure_code))
    {
        return Err("U4c production protected-terminal semantic evidence drifted".to_owned());
    }
    validate_rich_failure_evidence(
        &specification.ledger_contract,
        &report.evidence,
        specification
            .ledger_contract
            .rich_failure_protected_attempted_work,
        specification
            .ledger_contract
            .rich_failure_protected_consumed_work,
        &expected.scenario_id,
    )?;
    validate_producer_bound(
        &report.construction,
        "U4c production protected-terminal construction",
    )?;
    validate_zero_allocation_handoff(&report.handoff, "U4c production protected-terminal handoff")?;
    validate_physical_handoff_preserved(
        &report.physical_account,
        "U4c production protected-terminal handoff",
    )?;
    validate_release(&report.release, "U4c production protected-terminal release")?;
    validate_healthy_account(
        &report.physical_account.baseline,
        "U4c production protected-terminal baseline",
    )?;
    validate_healthy_account(
        &report.physical_account.retained,
        "U4c production protected-terminal retained",
    )?;
    validate_healthy_account(
        &report.physical_account.after_handoff,
        "U4c production protected-terminal handoff",
    )?;
    validate_released_account(
        &report.physical_account.after_release,
        "U4c production protected-terminal release",
    )?;
    validate_released_factory(
        &report.factory_after_account_release,
        "U4c production protected-terminal factory release",
    )
}

#[cfg(feature = "repository-input-candidate")]
fn validate_expected_producer_phase(
    report: &PhaseReport,
    expected: &ScaleScenarioSpecification,
    label: &str,
) -> Result<(), String> {
    let taxonomy = &report.taxonomy;
    if taxonomy.semantic_cell_writes != expected.semantic_cells
        || taxonomy.reference_writes != expected.references
        || taxonomy.block_begins != expected.blocks
        || taxonomy.block_seals != expected.blocks
        || taxonomy.slot_publications != expected.blocks
        || taxonomy.new_block_begins != expected.blocks
        || taxonomy.new_block_seals != expected.blocks
        || taxonomy.new_slot_publications != expected.blocks
        || taxonomy.inherited_block_begins != 0
        || taxonomy.inherited_block_seals != 0
        || taxonomy.inherited_slot_publications != 0
        || taxonomy.producer_events != expected.producer_events
    {
        return Err(format!("{label} producer transcript drifted"));
    }
    validate_producer_bound(report, label)
}

#[cfg(feature = "repository-input-candidate")]
fn validate_producer_bound(report: &PhaseReport, label: &str) -> Result<(), String> {
    let taxonomy = &report.taxonomy;
    if report.scenario_id.is_empty()
        || taxonomy.block_begins != taxonomy.block_seals
        || taxonomy.block_begins != taxonomy.slot_publications
        || taxonomy.new_block_begins != taxonomy.new_block_seals
        || taxonomy.new_block_begins != taxonomy.new_slot_publications
        || taxonomy.inherited_block_begins != taxonomy.inherited_block_seals
        || taxonomy.inherited_block_begins != taxonomy.inherited_slot_publications
        || taxonomy.block_begins
            != taxonomy
                .new_block_begins
                .checked_add(taxonomy.inherited_block_begins)
                .ok_or_else(|| format!("{label} block count overflowed"))?
    {
        return Err(format!("{label} block lifecycle drifted"));
    }
    let budget = taxonomy
        .semantic_cell_writes
        .checked_add(taxonomy.reference_writes)
        .and_then(|value| value.checked_add(taxonomy.block_begins.saturating_mul(3)))
        .ok_or_else(|| format!("{label} producer budget overflowed"))?;
    if taxonomy.producer_events > budget || taxonomy.element_decodes != 0 {
        return Err(format!("{label} violates producer or decode bounds"));
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
fn validate_inherited_share_phase(report: &PhaseReport, label: &str) -> Result<(), String> {
    let taxonomy = &report.taxonomy;
    if taxonomy.inherited_block_begins == 0
        || taxonomy.payload_backing_allocations != taxonomy.new_block_begins
    {
        return Err(format!(
            "{label} did not inherit a block or allocated backing outside new blocks"
        ));
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
fn validate_scale_lookup_phase(
    report: &PhaseReport,
    measurement: &MeasurementSpecification,
    label: &str,
) -> Result<(), String> {
    let taxonomy = &report.taxonomy;
    if taxonomy.completed_lookups != measurement.scale_completed_lookups
        || taxonomy.rejected_lookups != measurement.scale_rejected_lookups
        || taxonomy.lookup_events != measurement.scale_lookup_events
        || taxonomy.member_probes != measurement.scale_member_probes
    {
        return Err(format!(
            "{label} drifted from the frozen cross-scale lookup transcript: \
             observed completed/rejected/events/probes={}/{}/{}/{}, \
             expected={}/{}/{}/{}",
            taxonomy.completed_lookups,
            taxonomy.rejected_lookups,
            taxonomy.lookup_events,
            taxonomy.member_probes,
            measurement.scale_completed_lookups,
            measurement.scale_rejected_lookups,
            measurement.scale_lookup_events,
            measurement.scale_member_probes,
        ));
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
fn validate_hot_view_phase(
    report: &PhaseReport,
    measurement: &MeasurementSpecification,
    label: &str,
) -> Result<(), String> {
    let taxonomy = &report.taxonomy;
    let maximum_lookup_events = taxonomy
        .completed_lookups
        .checked_mul(measurement.max_lookup_events)
        .ok_or_else(|| format!("{label} lookup event envelope overflowed"))?;
    let maximum_member_probes = taxonomy
        .completed_lookups
        .checked_mul(measurement.max_member_probes)
        .ok_or_else(|| format!("{label} member-probe envelope overflowed"))?;
    if report.allocations.count_total > measurement.hot_view_max_allocations
        || report.allocations.count_max > measurement.hot_view_max_allocations
        || report.allocations.bytes_total > measurement.hot_view_max_allocations
        || report.allocations.bytes_max > measurement.hot_view_max_allocations
        || report.allocations.count_current != 0
        || report.allocations.bytes_current != 0
        || taxonomy.payload_copy_bytes > measurement.hot_view_max_payload_copy_bytes
        || taxonomy.payload_backing_allocations
            > measurement.hot_view_max_payload_backing_allocations
        || taxonomy.element_decodes > measurement.hot_view_max_element_decodes
        || taxonomy.max_lookup_events > measurement.max_lookup_events
        || taxonomy.max_member_probes_per_lookup > measurement.max_member_probes
        || taxonomy.rejected_lookups > taxonomy.completed_lookups
        || taxonomy.lookup_events < taxonomy.completed_lookups
        || taxonomy.lookup_events > maximum_lookup_events
        || taxonomy.member_probes > maximum_member_probes
        || taxonomy.max_lookup_events > taxonomy.lookup_events
        || taxonomy.member_probes > taxonomy.lookup_events
        || taxonomy.max_member_probes_per_lookup > taxonomy.member_probes
        || taxonomy.completed_lookups == 0
        || report.scenario_id.is_empty()
    {
        return Err(format!("{label} violates the frozen hot-view budget"));
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
fn validate_success_evidence(
    contract: &LedgerContractSpecification,
    evidence: &EvidenceReport,
    expected_canonical_block_count: u64,
    label: &str,
) -> Result<(), String> {
    let header = &evidence.ledger_header;
    if evidence.retained_evidence_block_count == 0
        || header.schema_revision != 1
        || header.terminal_cause.class != "success"
        || header.terminal_cause.failure_class.is_some()
        || header.terminal_cause.code.is_some()
        || header.dimension_count != 2
        || header.phase_count == 0
        || header.current_live_bytes != 0
        || header.active_reservations != 0
        || header.current_evidence_bytes != 0
        || header.peak_live_bytes == 0
        || header.peak_evidence_bytes == 0
    {
        return Err(format!("{label} success evidence drifted"));
    }
    let dimensions = validate_success_dimensions(contract, &evidence.dimensions, label)?;
    let expected_published_outcome_evidence = evidence
        .retained_evidence_block_count
        .checked_add(1)
        .ok_or_else(|| format!("{label} outcome evidence count overflowed"))?;
    if dimensions.ordinary.committed != expected_canonical_block_count
        || dimensions.ordinary.committed_canonical_output != expected_canonical_block_count
    {
        return Err(format!(
            "{label} canonical commitment drifted: expected {expected_canonical_block_count}, found committed={} canonical={}",
            dimensions.ordinary.committed, dimensions.ordinary.committed_canonical_output,
        ));
    }
    if dimensions.ordinary.published_outcome_evidence != expected_published_outcome_evidence {
        return Err(format!(
            "{label} ordinary outcome evidence drifted: expected {expected_published_outcome_evidence}, found {}",
            dimensions.ordinary.published_outcome_evidence
        ));
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
fn validate_rich_failure_evidence(
    contract: &LedgerContractSpecification,
    evidence: &EvidenceReport,
    expected_protected_attempted_work: u64,
    expected_protected_consumed_work: u64,
    label: &str,
) -> Result<(), String> {
    let header = &evidence.ledger_header;
    if evidence.retained_evidence_block_count == 0
        || header.schema_revision != 1
        || header.terminal_cause.class != "failure"
        || header.terminal_cause.failure_class.is_none()
        || header.terminal_cause.code.is_none()
        || header.dimension_count != 2
        || header.phase_count == 0
        || header.current_live_bytes != 0
        || header.active_reservations != 0
        || header.current_evidence_bytes != 0
        || header.peak_live_bytes == 0
        || header.peak_evidence_bytes == 0
    {
        return Err(format!("{label} failure evidence drifted"));
    }
    let dimensions = validate_rich_failure_dimensions(
        contract,
        &evidence.dimensions,
        expected_protected_attempted_work,
        expected_protected_consumed_work,
        label,
    )?;
    let expected_published_outcome_evidence = evidence
        .retained_evidence_block_count
        .checked_add(1)
        .ok_or_else(|| format!("{label} retained evidence count overflowed"))?;
    if dimensions.ordinary.published_outcome_evidence != expected_published_outcome_evidence {
        return Err(format!(
            "{label} ordinary failure evidence drifted: expected {}, found {}",
            expected_published_outcome_evidence, dimensions.ordinary.published_outcome_evidence
        ));
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
fn validate_minimal_fallback_evidence(
    contract: &LedgerContractSpecification,
    evidence: &EvidenceReport,
    expected_fallback_output_count: u64,
    label: &str,
) -> Result<(), String> {
    let header = &evidence.ledger_header;
    if evidence.retained_evidence_block_count != 0
        || expected_fallback_output_count == 0
        || header.schema_revision != 1
        || header.terminal_cause.class != "failure"
        || header.terminal_cause.failure_class.is_none()
        || header.terminal_cause.code.is_none()
        || header.dimension_count != 2
        || header.phase_count == 0
        || header.current_live_bytes != 0
        || header.active_reservations != 0
        || header.current_evidence_bytes != 0
        || header.peak_live_bytes == 0
        || header.peak_evidence_bytes == 0
    {
        return Err(format!("{label} minimal-fallback evidence drifted"));
    }
    let dimensions = validate_minimal_fallback_dimensions(
        contract,
        &evidence.dimensions,
        expected_fallback_output_count
            .checked_add(1)
            .ok_or_else(|| format!("{label} fallback publication count overflowed"))?,
        label,
    )?;
    let expected_published_outcome_evidence = expected_fallback_output_count
        .checked_add(1)
        .ok_or_else(|| format!("{label} fallback publication count overflowed"))?;
    if dimensions.ordinary.published_outcome_evidence != expected_published_outcome_evidence {
        return Err(format!(
            "{label} minimal-fallback commitment drifted: expected {expected_published_outcome_evidence}, found {}",
            dimensions.ordinary.published_outcome_evidence
        ));
    }
    Ok(())
}

fn classify_ledger_dimensions<'report>(
    contract: &LedgerContractSpecification,
    dimensions: &'report [LedgerDimensionReport],
    label: &str,
) -> Result<LedgerDimensionsByRole<'report>, String> {
    if dimensions.len() != 2 {
        return Err(format!(
            "{label} must enumerate exactly ordinary and failure-terminal ledger dimensions"
        ));
    }

    let mut semantic_ids = BTreeSet::new();
    let mut ordinary = None;
    let mut failure_terminal = None;
    for dimension in dimensions {
        if !dimension.has_terminal_invariants()
            || !semantic_ids.insert(dimension.semantic_id_hex.as_str())
        {
            return Err(format!("{label} ledger dimension evidence drifted"));
        }
        let (expected_semantic_id, slot) = match dimension.role {
            LedgerDimensionRole::Ordinary => {
                (contract.ordinary_semantic_id_hex.as_str(), &mut ordinary)
            }
            LedgerDimensionRole::FailureTerminal => (
                contract.failure_terminal_semantic_id_hex.as_str(),
                &mut failure_terminal,
            ),
        };
        if dimension.semantic_id_hex != expected_semantic_id || slot.replace(dimension).is_some() {
            return Err(format!(
                "{label} ledger dimension role/semantic identity drifted"
            ));
        }
    }

    Ok(LedgerDimensionsByRole {
        ordinary: ordinary
            .ok_or_else(|| format!("{label} ordinary ledger dimension is missing"))?,
        failure_terminal: failure_terminal
            .ok_or_else(|| format!("{label} failure-terminal ledger dimension is missing"))?,
    })
}

fn validate_success_dimensions<'report>(
    contract: &LedgerContractSpecification,
    dimensions: &'report [LedgerDimensionReport],
    label: &str,
) -> Result<LedgerDimensionsByRole<'report>, String> {
    let dimensions = classify_ledger_dimensions(contract, dimensions, label)?;
    if dimensions.ordinary.attempted == 0
        || dimensions.ordinary.attempted != dimensions.ordinary.consumed
        || dimensions.ordinary.peak_live_bytes == 0
        || dimensions.ordinary.committed == 0
        || dimensions.ordinary.committed_canonical_output != dimensions.ordinary.committed
        || dimensions.ordinary.published_outcome_evidence == 0
        || !dimensions.failure_terminal.is_unused_failure_terminal()
    {
        return Err(format!(
            "{label} success ledger dimensions violate the ordinary/protected contract"
        ));
    }
    Ok(dimensions)
}

fn validate_rich_failure_dimensions<'report>(
    contract: &LedgerContractSpecification,
    dimensions: &'report [LedgerDimensionReport],
    expected_protected_attempted_work: u64,
    expected_protected_consumed_work: u64,
    label: &str,
) -> Result<LedgerDimensionsByRole<'report>, String> {
    let dimensions = classify_ledger_dimensions(contract, dimensions, label)?;
    if dimensions.ordinary.committed != 0
        || dimensions.ordinary.committed_canonical_output != 0
        || dimensions.ordinary.published_outcome_evidence == 0
        || !dimensions.failure_terminal.matches_protected_terminal(
            expected_protected_attempted_work,
            expected_protected_consumed_work,
        )
    {
        return Err(format!(
            "{label} rich-failure ledger dimensions violate the ordinary/protected contract"
        ));
    }
    Ok(dimensions)
}

fn validate_minimal_fallback_dimensions<'report>(
    contract: &LedgerContractSpecification,
    dimensions: &'report [LedgerDimensionReport],
    expected_published_outcome_evidence: u64,
    label: &str,
) -> Result<LedgerDimensionsByRole<'report>, String> {
    let dimensions = classify_ledger_dimensions(contract, dimensions, label)?;
    if dimensions.ordinary.committed != 0
        || dimensions.ordinary.committed_canonical_output != 0
        || dimensions.ordinary.published_outcome_evidence != expected_published_outcome_evidence
        || !dimensions.failure_terminal.matches_protected_terminal(
            contract.minimal_fallback_protected_attempted_work,
            contract.minimal_fallback_protected_consumed_work,
        )
    {
        return Err(format!(
            "{label} minimal-fallback ledger dimensions violate the ordinary/protected contract"
        ));
    }
    Ok(dimensions)
}

#[cfg(feature = "repository-input-candidate")]
fn validate_zero_allocation_handoff(report: &HandoffReport, label: &str) -> Result<(), String> {
    if report.scenario_id.is_empty() || report.allocations != AllocationReport::default() {
        return Err(format!(
            "{label} must be a zero-work, zero-allocation ownership move"
        ));
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
fn validate_physical_handoff_preserved(
    report: &FaultPhysicalReport,
    label: &str,
) -> Result<(), String> {
    if report.retained != report.after_handoff {
        return Err(format!(
            "{label} changed physical account ownership during handoff"
        ));
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
fn validate_release(report: &ReleaseReport, label: &str) -> Result<(), String> {
    if report.scenario_id.is_empty()
        || report.allocations.count_total != 0
        || report.allocations.bytes_total != 0
        || report.allocations.count_max != 0
        || report.allocations.bytes_max != 0
        || report.allocations.count_current >= 0
        || report.allocations.bytes_current >= 0
    {
        return Err(format!(
            "{label} does not release retained allocation observations"
        ));
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
fn validate_healthy_account(snapshot: &PhysicalAccountReport, label: &str) -> Result<(), String> {
    if snapshot.poisoned
        || snapshot.poison_event_count != 0
        || !snapshot.poison_event_count_exact
        || !snapshot.poison_event_domain_valid
        || snapshot.peak_retained_bytes < snapshot.live_retained_bytes
        || snapshot.peak_retained_allocations < snapshot.live_retained_allocations
        || snapshot.total_admitted_retained_bytes < snapshot.live_retained_bytes
        || snapshot.total_admitted_retained_allocations < snapshot.live_retained_allocations
    {
        return Err(format!("{label} physical account evidence drifted"));
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
fn live_retained_charge(snapshot: &PhysicalAccountReport) -> (u64, u64) {
    (
        snapshot.live_retained_bytes,
        snapshot.live_retained_allocations,
    )
}

#[cfg(feature = "repository-input-candidate")]
fn validate_released_account(snapshot: &PhysicalAccountReport, label: &str) -> Result<(), String> {
    validate_healthy_account(snapshot, label)?;
    if snapshot.live_retained_bytes != 0 || snapshot.live_retained_allocations != 0 {
        return Err(format!("{label} retained physical charge was not released"));
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
fn validate_released_factory(snapshot: &PhysicalFactoryReport, label: &str) -> Result<(), String> {
    if snapshot.live_account_control_bytes != 0
        || snapshot.live_accounts != 0
        || snapshot.peak_account_control_bytes == 0
        || snapshot.peak_live_accounts == 0
        || snapshot.total_admitted_account_control_bytes == 0
        || snapshot.total_admitted_accounts == 0
    {
        return Err(format!("{label} factory account lifecycle drifted"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_the_canonical_specification_path() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");

        assert!(is_canonical_specification_path(
            &root,
            Path::new(SPECIFICATION_PATH)
        ));
        assert!(is_canonical_specification_path(
            &root,
            &root.join(SPECIFICATION_PATH)
        ));
        assert!(!is_canonical_specification_path(
            &root,
            Path::new("docs/conformance/other.toml")
        ));
        assert!(!is_canonical_specification_path(
            &root,
            Path::new("docs/conformance/../u4c-metrics.toml")
        ));
    }

    #[test]
    fn parses_the_managed_production_specification() {
        let specification = ProductionSpecification::parse(include_bytes!(
            "../../../../docs/conformance/u4c-metrics.toml"
        ))
        .expect("managed U4c production specification must parse");

        assert_eq!(
            required_capture_root_count(&specification.closure),
            Ok(4),
            "the managed closure has one specification, one workspace source tree, and two fixed files"
        );
    }

    #[test]
    fn rejects_retired_v1_production_contracts() {
        let managed = std::str::from_utf8(include_bytes!(
            "../../../../docs/conformance/u4c-metrics.toml"
        ))
        .expect("managed U4c production specification must be UTF-8");

        for retired in [
            managed.replace("schema_version = 7", "schema_version = 6"),
            managed.replace(
                "canonical_store_u4c_production_v2_probe",
                "canonical_store_u4c_production_v1_probe",
            ),
            managed.replace("BAOZI_U4C_PRODUCTION_V2=", "BAOZI_U4C_PRODUCTION_V1="),
            managed.replace("schema_version = 2", "schema_version = 1"),
            managed.replace("report_revision = 3", "report_revision = 10"),
            managed.replace(
                "scenario_set_id = \"canonical-store-production-v2\"",
                "scenario_set_id = \"canonical-store-production-v1\"",
            ),
        ] {
            assert!(
                ProductionSpecification::parse(retired.as_bytes()).is_err(),
                "retired V1 production contracts must not remain readable"
            );
        }
    }

    #[test]
    fn rejects_ledger_vocabulary_and_role_drift() {
        let managed = std::str::from_utf8(include_bytes!(
            "../../../../docs/conformance/u4c-metrics.toml"
        ))
        .expect("managed U4c production specification must be UTF-8");
        let ordinary_id = ORDINARY_DIMENSION_SEMANTIC_ID_HEX;
        let first_failure_id = managed
            .find(FAILURE_TERMINAL_DIMENSION_SEMANTIC_ID_HEX)
            .expect("managed ledger contract must contain the protected semantic identity");
        let second_failure_id = managed
            [first_failure_id + FAILURE_TERMINAL_DIMENSION_SEMANTIC_ID_HEX.len()..]
            .find(FAILURE_TERMINAL_DIMENSION_SEMANTIC_ID_HEX)
            .map(|offset| {
                first_failure_id + FAILURE_TERMINAL_DIMENSION_SEMANTIC_ID_HEX.len() + offset
            })
            .expect("managed changed SceneEdit must freeze the protected semantic identity");
        let mut changed_edit_identity_drift = managed.to_owned();
        changed_edit_identity_drift.replace_range(
            second_failure_id..second_failure_id + FAILURE_TERMINAL_DIMENSION_SEMANTIC_ID_HEX.len(),
            ordinary_id,
        );

        for malformed in [
            managed.replace(
                "rich_failure_protected_attempted_work = 6",
                "rich_failure_protected_attempted_work = 0",
            ),
            managed.replace(
                "minimal_fallback_protected_attempted_work = 0",
                "minimal_fallback_protected_attempted_work = 1",
            ),
            managed.replacen(FAILURE_TERMINAL_DIMENSION_SEMANTIC_ID_HEX, ordinary_id, 1),
            changed_edit_identity_drift,
            managed.replace("role = \"failure-terminal\"", "role = \"ordinary\""),
            managed.replace("role = \"failure-terminal\"", "role = \"unknown\""),
        ] {
            assert!(
                ProductionSpecification::parse(malformed.as_bytes()).is_err(),
                "managed production evidence must reject ledger vocabulary drift"
            );
        }
    }

    #[test]
    fn rejects_minimal_fallback_contract_drift() {
        let managed = std::str::from_utf8(include_bytes!(
            "../../../../docs/conformance/u4c-metrics.toml"
        ))
        .expect("managed U4c production specification must be UTF-8");

        let (before_minimal_fallback, minimal_fallback_and_after) = managed
            .split_once("[minimal_fallback]")
            .expect("managed production specification must contain minimal_fallback");
        let minimal_failure_class_drift = format!(
            "{before_minimal_fallback}[minimal_fallback]{}",
            minimal_fallback_and_after.replacen(
                "terminal_failure_class = \"ledger\"",
                "terminal_failure_class = \"requested\"",
                1,
            )
        );
        let error = ProductionSpecification::parse(minimal_failure_class_drift.as_bytes())
            .expect_err("minimal-fallback failure-class drift must fail closed");
        assert!(
            error.contains("minimal_fallback"),
            "failure-class drift must be rejected by the minimal-fallback contract: {error}"
        );

        for malformed in [
            managed.replace(
                "scenario_id = \"canonical-node-minimal-fallback-v1\"",
                "scenario_id = \"retired-minimal-fallback\"",
            ),
            managed.replace("fallback_code = 1396899841", "fallback_code = 0"),
            managed.replace("fallback_output_count = 1", "fallback_output_count = 2"),
        ] {
            assert!(
                ProductionSpecification::parse(malformed.as_bytes()).is_err(),
                "managed production evidence must reject minimal-fallback contract drift"
            );
        }
    }

    #[test]
    fn rejects_protected_terminal_activation_contract_drift() {
        let managed = std::str::from_utf8(include_bytes!(
            "../../../../docs/conformance/u4c-metrics.toml"
        ))
        .expect("managed U4c production specification must be UTF-8");

        for malformed in [
            managed.replace(
                "scenario_id = \"canonical-node-protected-terminal-activation-v1\"",
                "scenario_id = \"unprotected-terminal-activation\"",
            ),
            managed.replace(
                "terminal_failure_code = 1\nretained_evidence_blocks = 1",
                "terminal_failure_code = 1\nretained_evidence_blocks = 0",
            ),
        ] {
            assert!(
                ProductionSpecification::parse(malformed.as_bytes()).is_err(),
                "managed production evidence must reject protected-terminal activation drift"
            );
        }
    }

    #[test]
    fn rejects_pending_scene_activation_contract_drift() {
        let managed = std::str::from_utf8(include_bytes!(
            "../../../../docs/conformance/u4c-metrics.toml"
        ))
        .expect("managed U4c production specification must be UTF-8");

        for malformed in [
            managed.replace(
                "entry_point = \"pending-scene-activation\"",
                "entry_point = \"retired-pending-activation\"",
            ),
            managed.replace(
                "scenario_id = \"canonical-node-pending-scene-activation-v1\"",
                "scenario_id = \"retired-pending-activation\"",
            ),
            managed.replace(
                "max_payload_copy_bytes = 1528",
                "max_payload_copy_bytes = 0",
            ),
        ] {
            assert!(
                ProductionSpecification::parse(malformed.as_bytes()).is_err(),
                "managed production evidence must reject pending activation contract drift"
            );
        }
    }

    #[test]
    fn ledger_dimension_roles_enforce_success_rich_and_minimal_failure_contracts() {
        let contract = LedgerContractSpecification {
            ordinary_semantic_id_hex: ORDINARY_DIMENSION_SEMANTIC_ID_HEX.to_owned(),
            failure_terminal_semantic_id_hex: FAILURE_TERMINAL_DIMENSION_SEMANTIC_ID_HEX.to_owned(),
            rich_failure_protected_attempted_work: 6,
            rich_failure_protected_consumed_work: 6,
            minimal_fallback_protected_attempted_work: 0,
            minimal_fallback_protected_consumed_work: 0,
        };
        let ordinary_success = LedgerDimensionReport {
            role: LedgerDimensionRole::Ordinary,
            semantic_id_hex: ORDINARY_DIMENSION_SEMANTIC_ID_HEX.to_owned(),
            attempted: 8,
            consumed: 8,
            current_live_bytes: 0,
            peak_live_bytes: 64,
            pending_publication: 0,
            committed: 2,
            committed_canonical_output: 2,
            published_outcome_evidence: 2,
        };
        let protected_unused = LedgerDimensionReport {
            role: LedgerDimensionRole::FailureTerminal,
            semantic_id_hex: FAILURE_TERMINAL_DIMENSION_SEMANTIC_ID_HEX.to_owned(),
            attempted: 0,
            consumed: 0,
            current_live_bytes: 0,
            peak_live_bytes: 0,
            pending_publication: 0,
            committed: 0,
            committed_canonical_output: 0,
            published_outcome_evidence: 0,
        };
        let mut success = vec![ordinary_success, protected_unused];
        assert!(validate_success_dimensions(&contract, &success, "test success").is_ok());
        success.reverse();
        assert!(validate_success_dimensions(&contract, &success, "test success").is_ok());
        success.reverse();
        assert!(validate_success_dimensions(&contract, &success[..1], "test success").is_err());

        success[0].consumed = 7;
        assert!(validate_success_dimensions(&contract, &success, "test success").is_err());
        success[0].consumed = 8;
        success[1].attempted = 1;
        assert!(validate_success_dimensions(&contract, &success, "test success").is_err());

        let ordinary_failure = LedgerDimensionReport {
            role: LedgerDimensionRole::Ordinary,
            semantic_id_hex: ORDINARY_DIMENSION_SEMANTIC_ID_HEX.to_owned(),
            attempted: 8,
            consumed: 8,
            current_live_bytes: 0,
            peak_live_bytes: 64,
            pending_publication: 0,
            committed: 0,
            committed_canonical_output: 0,
            published_outcome_evidence: 2,
        };
        let protected_failure = LedgerDimensionReport {
            role: LedgerDimensionRole::FailureTerminal,
            semantic_id_hex: FAILURE_TERMINAL_DIMENSION_SEMANTIC_ID_HEX.to_owned(),
            attempted: 6,
            consumed: 6,
            current_live_bytes: 0,
            peak_live_bytes: 0,
            pending_publication: 0,
            committed: 0,
            committed_canonical_output: 0,
            published_outcome_evidence: 0,
        };
        let mut failure = vec![ordinary_failure, protected_failure];
        assert!(
            validate_rich_failure_dimensions(&contract, &failure, 6, 6, "test failure").is_ok()
        );

        failure[1].consumed = 5;
        assert!(
            validate_rich_failure_dimensions(&contract, &failure, 6, 6, "test failure").is_err()
        );
        failure[1].consumed = 6;
        failure[1].peak_live_bytes = 1;
        assert!(
            validate_rich_failure_dimensions(&contract, &failure, 6, 6, "test failure").is_err()
        );
        failure[1].peak_live_bytes = 0;
        failure[1].published_outcome_evidence = 1;
        assert!(
            validate_rich_failure_dimensions(&contract, &failure, 6, 6, "test failure").is_err()
        );
        failure[1].published_outcome_evidence = 0;
        failure[1].role = LedgerDimensionRole::Ordinary;
        assert!(
            validate_rich_failure_dimensions(&contract, &failure, 6, 6, "test failure").is_err()
        );

        failure[1].role = LedgerDimensionRole::FailureTerminal;
        failure[1].attempted = 0;
        failure[1].consumed = 0;
        failure[0].published_outcome_evidence = 2;
        assert!(
            validate_minimal_fallback_dimensions(&contract, &failure, 2, "test fallback").is_ok()
        );
        failure[1].attempted = 1;
        assert!(
            validate_minimal_fallback_dimensions(&contract, &failure, 2, "test fallback").is_err()
        );
    }

    #[cfg(feature = "repository-input-candidate")]
    #[test]
    fn evidence_validators_bind_header_cardinality_commitments_and_snapshot_publications() {
        let contract = LedgerContractSpecification {
            ordinary_semantic_id_hex: ORDINARY_DIMENSION_SEMANTIC_ID_HEX.to_owned(),
            failure_terminal_semantic_id_hex: FAILURE_TERMINAL_DIMENSION_SEMANTIC_ID_HEX.to_owned(),
            rich_failure_protected_attempted_work: 6,
            rich_failure_protected_consumed_work: 6,
            minimal_fallback_protected_attempted_work: 0,
            minimal_fallback_protected_consumed_work: 0,
        };
        let mut evidence = EvidenceReport {
            retained_evidence_block_count: 1,
            ledger_header: LedgerHeaderReport {
                schema_revision: 1,
                terminal_cause: TerminalCauseReport {
                    class: "success".to_owned(),
                    failure_class: None,
                    code: None,
                },
                dimension_count: 2,
                phase_count: 1,
                current_live_bytes: 0,
                peak_live_bytes: 64,
                active_reservations: 0,
                current_evidence_bytes: 0,
                peak_evidence_bytes: 64,
            },
            dimensions: vec![
                LedgerDimensionReport {
                    role: LedgerDimensionRole::Ordinary,
                    semantic_id_hex: ORDINARY_DIMENSION_SEMANTIC_ID_HEX.to_owned(),
                    attempted: 8,
                    consumed: 8,
                    current_live_bytes: 0,
                    peak_live_bytes: 64,
                    pending_publication: 0,
                    committed: 2,
                    committed_canonical_output: 2,
                    published_outcome_evidence: 2,
                },
                LedgerDimensionReport {
                    role: LedgerDimensionRole::FailureTerminal,
                    semantic_id_hex: FAILURE_TERMINAL_DIMENSION_SEMANTIC_ID_HEX.to_owned(),
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
        };

        assert!(validate_success_evidence(&contract, &evidence, 2, "test success").is_ok());
        assert!(validate_success_evidence(&contract, &evidence, 3, "test success").is_err());

        evidence.dimensions[0].published_outcome_evidence = 1;
        assert!(
            validate_success_evidence(&contract, &evidence, 2, "test success").is_err(),
            "success evidence must publish one ledger snapshot beyond its retained block"
        );
        evidence.dimensions[0].published_outcome_evidence = 2;

        evidence.ledger_header.terminal_cause = TerminalCauseReport {
            class: "failure".to_owned(),
            failure_class: Some("ledger".to_owned()),
            code: Some(1),
        };
        evidence.dimensions[0].committed = 0;
        evidence.dimensions[0].committed_canonical_output = 0;
        evidence.dimensions[1].attempted = 6;
        evidence.dimensions[1].consumed = 6;
        assert!(validate_rich_failure_evidence(&contract, &evidence, 6, 6, "test failure").is_ok());
        evidence.dimensions[0].published_outcome_evidence = 1;
        assert!(
            validate_rich_failure_evidence(&contract, &evidence, 6, 6, "test failure").is_err(),
            "rich failure evidence must publish one ledger snapshot beyond its retained block"
        );

        evidence.retained_evidence_block_count = 0;
        evidence.dimensions[1].attempted = 0;
        evidence.dimensions[1].consumed = 0;
        assert!(
            validate_minimal_fallback_evidence(&contract, &evidence, 1, "test fallback").is_err(),
            "minimal fallback evidence must publish one ledger snapshot beyond its bounded output"
        );
        evidence.dimensions[0].published_outcome_evidence = 2;
        assert!(
            validate_minimal_fallback_evidence(&contract, &evidence, 1, "test fallback").is_ok()
        );

        evidence.ledger_header.dimension_count = 1;
        assert!(
            validate_minimal_fallback_evidence(&contract, &evidence, 1, "test fallback").is_err()
        );
    }

    #[cfg(feature = "repository-input-candidate")]
    #[test]
    fn handoff_evidence_rejects_allocation_and_physical_account_drift() {
        let mut handoff = HandoffReport {
            scenario_id: "test-handoff".to_owned(),
            allocations: AllocationReport::default(),
        };
        assert!(validate_zero_allocation_handoff(&handoff, "test handoff").is_ok());
        handoff.allocations.count_total = 1;
        let error = validate_zero_allocation_handoff(&handoff, "test handoff")
            .expect_err("handoff allocation must fail closed");
        assert!(error.contains("zero-allocation"));

        let mut physical = FaultPhysicalReport::default();
        assert!(validate_physical_handoff_preserved(&physical, "test handoff").is_ok());
        physical.after_handoff.live_retained_bytes = 1;
        let error = validate_physical_handoff_preserved(&physical, "test handoff")
            .expect_err("physical ownership drift must fail closed");
        assert!(error.contains("physical account ownership"));
    }

    #[test]
    fn rejects_authored_exit_qualification() {
        let managed = std::str::from_utf8(include_bytes!(
            "../../../../docs/conformance/u4c-metrics.toml"
        ))
        .expect("managed U4c production specification must be UTF-8");

        for authored_field in [
            "declared_private_tracer_mapping_complete = true",
            "qualifies_u4d_exit = true",
            "qualifies_u4_exit = true",
        ] {
            let authored = format!("{authored_field}\n{managed}");
            assert!(
                ProductionSpecification::parse(authored.as_bytes()).is_err(),
                "qualification must be derived by the comparator, not authored as {authored_field:?}"
            );
        }
    }

    #[test]
    fn rejects_coverage_mapping_to_an_untyped_scenario() {
        let managed = std::str::from_utf8(include_bytes!(
            "../../../../docs/conformance/u4c-metrics.toml"
        ))
        .expect("managed U4c production specification must be UTF-8");
        let mapped = managed.replace(
            "{ entry_point = \"canonical-obj-corner-smoothing\", scenario_id = \"canonical-obj-corner-smoothing-v1\" }",
            "{ entry_point = \"canonical-obj-corner-smoothing\", scenario_id = \"complex-members-n-v1\" }",
        );

        let error = ProductionSpecification::parse(mapped.as_bytes())
            .expect_err("synthetic coverage mapping must be rejected");
        assert!(error.contains("coverage mapping"), "{error}");
    }

    #[test]
    fn rejects_missing_obj_coverage_mapping() {
        let managed = std::str::from_utf8(include_bytes!(
            "../../../../docs/conformance/u4c-metrics.toml"
        ))
        .expect("managed U4c production specification must be UTF-8");
        let unmapped = managed.replace(
            "    { entry_point = \"canonical-obj-corner-smoothing\", scenario_id = \"canonical-obj-corner-smoothing-v1\" },\n",
            "",
        );

        let error = ProductionSpecification::parse(unmapped.as_bytes())
            .expect_err("the typed OBJ scenario must remain mapped");
        assert!(error.contains("OBJ coverage mapping"), "{error}");
    }

    #[test]
    fn rejects_missing_private_tracer_mapping() {
        let managed = std::str::from_utf8(include_bytes!(
            "../../../../docs/conformance/u4c-metrics.toml"
        ))
        .expect("managed U4c production specification must be UTF-8");
        let unmapped = managed.replace(
            "    { entry_point = \"facade-equivalent-inspection\", scenario_id = \"canonical-gltf-five-domain-inspection-v1\" },\n",
            "",
        );

        let error = ProductionSpecification::parse(unmapped.as_bytes())
            .expect_err("the declared private tracer set must remain complete");
        assert!(error.contains("private tracer coverage"), "{error}");
    }

    #[test]
    fn rejects_coverage_mapping_to_the_wrong_gltf_scenario() {
        let managed = std::str::from_utf8(include_bytes!(
            "../../../../docs/conformance/u4c-metrics.toml"
        ))
        .expect("managed U4c production specification must be UTF-8");
        let mapped = managed.replace(
            "{ entry_point = \"backend-free-gltf-shape\", scenario_id = \"backend-free-gltf-shape-v1\" }",
            "{ entry_point = \"backend-free-gltf-shape\", scenario_id = \"complex-members-n-v1\" }",
        );

        let error = ProductionSpecification::parse(mapped.as_bytes())
            .expect_err("synthetic glTF coverage mapping must be rejected");
        assert!(error.contains("glTF coverage mapping"), "{error}");
    }

    #[test]
    fn rejects_coverage_mapping_to_the_wrong_pending_activation_scenario() {
        let managed = std::str::from_utf8(include_bytes!(
            "../../../../docs/conformance/u4c-metrics.toml"
        ))
        .expect("managed U4c production specification must be UTF-8");
        let mapped = managed.replace(
            "{ entry_point = \"pending-scene-activation\", scenario_id = \"canonical-node-pending-scene-activation-v1\" }",
            "{ entry_point = \"pending-scene-activation\", scenario_id = \"wrong-pending-scenario-v1\" }",
        );

        let error = ProductionSpecification::parse(mapped.as_bytes())
            .expect_err("synthetic pending activation coverage mapping must be rejected");
        assert!(error.contains("pending activation coverage"), "{error}");
    }

    #[test]
    fn rejects_missing_pending_activation_coverage_mapping() {
        let managed = std::str::from_utf8(include_bytes!(
            "../../../../docs/conformance/u4c-metrics.toml"
        ))
        .expect("managed U4c production specification must be UTF-8");
        let unmapped = managed.replace(
            "    { entry_point = \"pending-scene-activation\", scenario_id = \"canonical-node-pending-scene-activation-v1\" },\n",
            "",
        );

        let error = ProductionSpecification::parse(unmapped.as_bytes())
            .expect_err("pending activation must remain mapped");
        assert!(error.contains("pending activation coverage"), "{error}");
    }

    #[test]
    fn complete_private_tracer_report_does_not_claim_u4d_exit() {
        let evidence_summary = ProductionEvidenceSummary::after_validated_private_tracer_report();
        assert!(evidence_summary.declared_private_tracer_mapping_complete);
        assert!(!evidence_summary.qualifies_u4d_exit);
    }

    #[test]
    fn rejects_consumer_coverage_mapping_drift() {
        let managed = std::str::from_utf8(include_bytes!(
            "../../../../docs/conformance/u4c-metrics.toml"
        ))
        .expect("managed U4c production specification must be UTF-8");
        for (entry_point, scenario_id) in [
            (
                "facade-equivalent-inspection",
                "canonical-gltf-five-domain-inspection-v1",
            ),
            ("bounded-snapshot", "canonical-gltf-five-domain-snapshot-v1"),
            (
                "unchanged-view",
                "canonical-gltf-five-domain-unchanged-view-v1",
            ),
            (
                "changed-scene-edit",
                "canonical-gltf-node-transform-edit-v1",
            ),
        ] {
            let expected =
                format!("{{ entry_point = \"{entry_point}\", scenario_id = \"{scenario_id}\" }}");
            let drifted = format!(
                "{{ entry_point = \"{entry_point}\", scenario_id = \"wrong-scenario-v1\" }}"
            );
            let malformed = managed.replace(&expected, &drifted);

            let error = ProductionSpecification::parse(malformed.as_bytes())
                .expect_err("consumer coverage mapping drift must be rejected");
            assert!(error.contains("coverage mapping"), "{error}");
        }
    }

    #[test]
    fn rejects_local_package_without_repository_manifest_identity() {
        let managed = std::str::from_utf8(include_bytes!(
            "../../../../docs/conformance/u4c-metrics.toml"
        ))
        .expect("managed U4c production specification must be UTF-8");
        let unbound = managed.replace(
            "repository_manifest = \"crates/baozi-core/Cargo.toml\"\n",
            "",
        );

        let error = ProductionSpecification::parse(unbound.as_bytes())
            .expect_err("local package without manifest identity must be rejected");
        assert!(error.contains("repository manifest"), "{error}");
    }

    #[test]
    fn rejects_cross_target_production_identity() {
        let managed = std::str::from_utf8(include_bytes!(
            "../../../../docs/conformance/u4c-metrics.toml"
        ))
        .expect("managed U4c production specification must be UTF-8");
        let cross_target = managed.replace(
            "target = \"x86_64-unknown-linux-gnu\"",
            "target = \"aarch64-apple-darwin\"",
        );

        assert!(
            ProductionSpecification::parse(cross_target.as_bytes()).is_err(),
            "the production probe must execute on its declared compiler host"
        );
    }

    #[test]
    fn rejects_specification_drift_between_capture_passes() {
        assert!(
            verify_bootstrap_specification_bytes(b"before", b"after").is_err(),
            "the complete closure must retain the exact specification parsed at bootstrap"
        );
        assert!(verify_bootstrap_specification_bytes(b"same", b"same").is_ok());
    }

    #[cfg(feature = "repository-input-candidate")]
    #[test]
    fn sanitizes_cargo_execution_environment_names_case_insensitively() {
        for name in [
            "CARGO_BUILD_TARGET",
            "cargo_build_target",
            "Cargo_Profile_Test_Opt_Level",
            "CARGO_ENCODED_RUSTFLAGS",
            "RUSTFLAGS",
            "rustflags",
            "Rustc_Wrapper",
        ] {
            assert!(
                is_sanitized_cargo_environment_name(name),
                "{name} must not reach the U4c Cargo child"
            );
        }
        assert!(!is_sanitized_cargo_environment_name("PATH"));
        assert!(is_sanitized_cargo_environment_name("RUSTUP_TOOLCHAIN"));
    }

    #[cfg(feature = "repository-input-candidate")]
    #[test]
    fn cargo_sandbox_uses_an_isolated_config_and_artifact_boundary() {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let toolchain = VerifiedToolchain {
            cargo: PathBuf::from("verified-cargo"),
            rustc: PathBuf::from("verified-rustc"),
            sysroot: repository_root.join("verified-sysroot"),
            target: "verified-target".to_owned(),
            file_count: 1,
            sha256: "verified-sha256".to_owned(),
        };
        let sandbox = CargoSandbox::create(&repository_root, toolchain)
            .expect("the managed repository must admit a Cargo sandbox");
        let command = sandbox.network_command();

        assert_eq!(
            command.get_program(),
            std::ffi::OsStr::new("verified-cargo")
        );
        assert_eq!(
            command.get_current_dir(),
            Some(sandbox.execution_directory.as_path())
        );
        assert_eq!(
            command
                .get_envs()
                .find(|(name, _)| *name == "CARGO_CACHE_RUSTC_INFO")
                .map(|(_, value)| value),
            Some(Some(std::ffi::OsStr::new("0")))
        );
        assert_eq!(
            command
                .get_envs()
                .find(|(name, _)| *name == "CARGO_HOME")
                .map(|(_, value)| value),
            Some(Some(sandbox.cargo_home.as_os_str()))
        );
        assert_eq!(
            command
                .get_envs()
                .find(|(name, _)| *name == "CARGO_TARGET_DIR")
                .map(|(_, value)| value),
            Some(Some(sandbox.target_dir.as_os_str()))
        );
        assert_eq!(
            command
                .get_envs()
                .find(|(name, _)| *name == "CARGO_BUILD_JOBS")
                .map(|(_, value)| value),
            Some(Some(std::ffi::OsStr::new("1")))
        );
        assert_eq!(
            command
                .get_envs()
                .find(|(name, _)| *name == "CARGO")
                .map(|(_, value)| value),
            Some(None)
        );
        assert_eq!(
            command
                .get_envs()
                .find(|(name, _)| *name == "RUSTC")
                .map(|(_, value)| value),
            Some(Some(std::ffi::OsStr::new("verified-rustc")))
        );
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["--config", "build.jobs = 1"].map(std::ffi::OsStr::new)
        );

        let build_command = sandbox
            .build_command()
            .expect("the managed sandbox must build a deterministic command");
        assert_eq!(
            build_command
                .get_envs()
                .find(|(name, _)| *name == "CARGO_CACHE_RUSTC_INFO")
                .map(|(_, value)| value),
            Some(Some(std::ffi::OsStr::new("0")))
        );
        let expected_rustflags = deterministic_encoded_rustflags(
            &sandbox.root,
            &sandbox.repository_root,
            &sandbox.toolchain.sysroot,
        )
        .expect("managed paths must have deterministic rustflags");
        assert_eq!(
            build_command
                .get_envs()
                .find(|(name, _)| *name == "CARGO_ENCODED_RUSTFLAGS")
                .map(|(_, value)| value),
            Some(Some(expected_rustflags.as_os_str()))
        );
    }

    #[cfg(feature = "repository-input-candidate")]
    #[test]
    fn rejects_lookup_aggregate_that_exceeds_per_lookup_probe_bound() {
        let specification = managed_specification();
        let mut phase = empty_phase_report();
        phase.taxonomy.completed_lookups = 1;
        phase.taxonomy.lookup_events = 8;
        phase.taxonomy.max_lookup_events = 8;
        phase.taxonomy.member_probes = 8;
        phase.taxonomy.max_member_probes_per_lookup = 1;

        assert!(
            validate_hot_view_phase(&phase, &specification.measurement, "malformed lookup")
                .is_err()
        );
    }

    #[cfg(feature = "repository-input-candidate")]
    #[test]
    fn freezes_the_scale_lookup_transcript() {
        let specification = managed_specification();
        let mut phase = empty_phase_report();
        phase.taxonomy.completed_lookups = 7;
        phase.taxonomy.rejected_lookups = 1;
        phase.taxonomy.lookup_events = 50;
        phase.taxonomy.max_lookup_events = 8;
        phase.taxonomy.member_probes = 6;
        phase.taxonomy.max_member_probes_per_lookup = 1;

        validate_hot_view_phase(&phase, &specification.measurement, "frozen lookup")
            .expect("the frozen aggregate must satisfy per-lookup envelopes");
        validate_scale_lookup_phase(&phase, &specification.measurement, "frozen lookup")
            .expect("the frozen aggregate must match the scale transcript");

        phase.taxonomy.member_probes = 5;
        assert!(
            validate_scale_lookup_phase(&phase, &specification.measurement, "drifted lookup")
                .is_err()
        );
    }

    #[cfg(feature = "repository-input-candidate")]
    #[test]
    fn inherited_share_accepts_new_block_copy_but_rejects_inherited_backing() {
        let mut phase = empty_phase_report();
        phase.taxonomy.new_block_begins = 2;
        phase.taxonomy.inherited_block_begins = 1;
        phase.taxonomy.payload_copy_bytes = 1;
        phase.taxonomy.payload_backing_allocations = 2;
        validate_inherited_share_phase(&phase, "inherited share")
            .expect("new blocks may copy payload, but only new blocks may allocate backing");

        phase.taxonomy.payload_backing_allocations = 3;
        assert!(validate_inherited_share_phase(&phase, "inherited share").is_err());
        phase.taxonomy.payload_backing_allocations = 2;
        phase.taxonomy.inherited_block_begins = 0;
        assert!(validate_inherited_share_phase(&phase, "inherited share").is_err());
    }

    #[cfg(feature = "repository-input-candidate")]
    fn managed_specification() -> ProductionSpecification {
        ProductionSpecification::parse(include_bytes!(
            "../../../../docs/conformance/u4c-metrics.toml"
        ))
        .expect("managed U4c production specification must parse")
    }

    #[cfg(feature = "repository-input-candidate")]
    fn empty_phase_report() -> PhaseReport {
        PhaseReport {
            scenario_id: "test-phase".to_owned(),
            taxonomy: MeasurementTaxonomyReport {
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
            },
            allocations: AllocationReport {
                count_total: 0,
                count_current: 0,
                count_max: 0,
                bytes_total: 0,
                bytes_current: 0,
                bytes_max: 0,
            },
        }
    }
}
