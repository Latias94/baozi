use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use baozi_xtask::conformance::{check_inventory_documents, check_repository};

const PARTIAL_CONTRACTS: &str = r#"
schema_version = 1

[[contracts]]
contract_id = "test-contract"
authority_adr = "docs/adr/0003-core-scene-ir-and-material-model.md"
conformance_status = "partial"
evidence_subject = "baozi.test.contract"
evidence_revision = 1
implementation_tracers = ["Cargo.toml"]
test_tracers = ["Cargo.lock"]
ci_tracers = [".github/workflows/ci.yml"]
gate_ids = ["test-gate"]
"#;

const PARTIAL_GATES: &str = r#"
schema_version = 1

[[gates]]
gate_id = "test-gate"
tracer_class = "test"
executor = "command"
success_condition = "process-exit-zero"
evidence_identity = "baozi.test.gate/v1"
evidence_paths = ["Cargo.lock"]
applicable_contracts = ["test-contract"]
program = "cargo"
args = ["test", "--locked"]
"#;

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("baozi-xtask must be below the repository crates directory")
        .to_path_buf()
}

fn assert_rejected(contracts: &str, gates: &str, expected: &str) {
    let report = check_inventory_documents(&repository_root(), contracts, gates);
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.contains(expected)),
        "expected {expected:?} in {:#?}",
        report.violations
    );
}

#[test]
fn checked_in_repository_inventory_is_structurally_valid() {
    let report = check_repository(&repository_root());
    assert!(report.violations.is_empty(), "{:#?}", report.violations);
    assert_eq!(report.contracts_checked, 12);
    assert_eq!(report.gates_checked, 13);
    assert_eq!(report.declared_conforming_contracts, 0);
    assert_eq!(report.self_attested_contract_bindings_matched, 0);

    let gates = fs::read_to_string(repository_root().join("docs/conformance/gates.toml"))
        .expect("checked-in conformance gates");
    assert!(gates.contains("gate_id = \"core-scene-integration-boundary-ci\""));
}

#[test]
fn partial_inventory_accepts_explicit_tracers_without_claiming_conformance() {
    let report = check_inventory_documents(&repository_root(), PARTIAL_CONTRACTS, PARTIAL_GATES);
    assert!(report.violations.is_empty(), "{:#?}", report.violations);
    assert_eq!(report.declared_conforming_contracts, 0);
}

#[test]
fn malformed_gate_document_preserves_declared_conforming_count() {
    let contracts = PARTIAL_CONTRACTS.replace(
        "conformance_status = \"partial\"",
        "conformance_status = \"conforming\"",
    );
    let report = check_inventory_documents(&repository_root(), &contracts, "not valid TOML = [");

    assert_eq!(report.contracts_checked, 1);
    assert_eq!(report.gates_checked, 0);
    assert_eq!(report.declared_conforming_contracts, 1);
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.contains("failed to parse")),
        "{:#?}",
        report.violations
    );
}

#[test]
fn in_memory_inventory_documents_keep_the_same_size_bound_as_repository_files() {
    let oversized = "x".repeat((256 * 1024) + 1);
    assert_rejected(&oversized, PARTIAL_GATES, "exceeds 262144 bytes");
    assert_rejected(PARTIAL_CONTRACTS, &oversized, "exceeds 262144 bytes");
}

#[test]
fn duplicate_and_unknown_gate_ids_fail_closed() {
    let duplicate = PARTIAL_CONTRACTS.replace(
        "gate_ids = [\"test-gate\"]",
        "gate_ids = [\"test-gate\", \"test-gate\"]",
    );
    assert_rejected(&duplicate, PARTIAL_GATES, "duplicate");

    let unknown = PARTIAL_CONTRACTS.replace("test-gate", "unknown-gate");
    assert_rejected(&unknown, PARTIAL_GATES, "unknown gate");
}

#[test]
fn duplicate_or_orphan_records_and_contradictory_applicability_fail_closed() {
    let (_, contract_record) = PARTIAL_CONTRACTS
        .split_once("[[contracts]]")
        .expect("test contract document must contain one record");
    let duplicate_contract = format!("{PARTIAL_CONTRACTS}\n[[contracts]]{contract_record}");
    assert_rejected(&duplicate_contract, PARTIAL_GATES, "duplicate contract_id");

    let (_, gate_record) = PARTIAL_GATES
        .split_once("[[gates]]")
        .expect("test gate document must contain one record");
    let duplicate_gate = format!("{PARTIAL_GATES}\n[[gates]]{gate_record}");
    assert_rejected(PARTIAL_CONTRACTS, &duplicate_gate, "duplicate gate_id");

    let orphan_gate = format!(
        "{PARTIAL_GATES}\n[[gates]]{}",
        gate_record.replace("gate_id = \"test-gate\"", "gate_id = \"orphan-gate\"")
    );
    assert_rejected(PARTIAL_CONTRACTS, &orphan_gate, "not referenced");

    let contradictory = PARTIAL_GATES.replace(
        "applicable_contracts = [\"test-contract\"]",
        "applicable_contracts = [\"other-contract\"]",
    );
    assert_rejected(
        PARTIAL_CONTRACTS,
        &contradictory,
        "does not declare contract",
    );

    let missing_reverse_reference =
        PARTIAL_CONTRACTS.replace("gate_ids = [\"test-gate\"]", "gate_ids = []");
    assert_rejected(
        &missing_reverse_reference,
        PARTIAL_GATES,
        "contract does not reference the gate",
    );
}

#[test]
fn explicit_paths_reject_escape_absolute_backslash_and_missing_files() {
    for malformed in [
        PARTIAL_CONTRACTS.replace("Cargo.toml", "../Cargo.toml"),
        PARTIAL_CONTRACTS.replace("Cargo.toml", "C:/outside/Cargo.toml"),
        PARTIAL_CONTRACTS.replace("Cargo.toml", "docs\\\\adr\\\\0003.md"),
        PARTIAL_CONTRACTS.replace(
            "Cargo.toml",
            "docs//adr/0003-core-scene-ir-and-material-model.md",
        ),
        PARTIAL_CONTRACTS.replace("Cargo.toml", "missing-explicit-tracer.rs"),
    ] {
        assert_rejected(&malformed, PARTIAL_GATES, "path");
    }

    let case_alias = PARTIAL_CONTRACTS.replace(
        "implementation_tracers = [\"Cargo.toml\"]",
        "implementation_tracers = [\"Cargo.toml\", \"CARGO.TOML\"]",
    );
    assert_rejected(&case_alias, PARTIAL_GATES, "portable path alias");

    for reserved in ["NUL.txt", "con", "Aux.md", "COM1.log", "lpt9"] {
        let malformed = PARTIAL_CONTRACTS.replace("Cargo.toml", reserved);
        assert_rejected(&malformed, PARTIAL_GATES, "reserved Windows device name");
    }
}

#[test]
fn conforming_status_requires_all_tracer_and_gate_classes() {
    let malformed = PARTIAL_CONTRACTS.replace(
        "conformance_status = \"partial\"",
        "conformance_status = \"conforming\"",
    );
    assert_rejected(&malformed, PARTIAL_GATES, "lacks complete tracers");
}

#[test]
fn conforming_status_requires_a_trusted_receipt_even_with_complete_declared_shape() {
    let contracts = PARTIAL_CONTRACTS
        .replace(
            "conformance_status = \"partial\"",
            "conformance_status = \"conforming\"",
        )
        .replace(
            "gate_ids = [\"test-gate\"]",
            "gate_ids = [\"implementation-gate\", \"test-gate\", \"ci-gate\"]",
        );
    let gates = r#"
schema_version = 1

[[gates]]
gate_id = "implementation-gate"
tracer_class = "implementation"
executor = "command"
success_condition = "process-exit-zero"
evidence_identity = "baozi.test.implementation/v1"
evidence_paths = ["Cargo.toml"]
applicable_contracts = ["test-contract"]
evidence_claims = [{ contract_id = "test-contract", evidence_subject = "baozi.test.contract", evidence_revision = 1 }]
program = "program-that-does-not-exist"
args = ["check", "--locked"]

[[gates]]
gate_id = "test-gate"
tracer_class = "test"
executor = "command"
success_condition = "process-exit-zero"
evidence_identity = "baozi.test.gate/v1"
evidence_paths = ["Cargo.lock"]
applicable_contracts = ["test-contract"]
program = "cargo"
args = ["test", "--locked"]

[[gates]]
gate_id = "ci-gate"
tracer_class = "ci"
executor = "workflow"
success_condition = "required-workflow-status"
evidence_identity = "github-actions/ci/rust/test"
evidence_paths = [".github/workflows/ci.yml"]
applicable_contracts = ["test-contract"]
workflow_path = ".github/workflows/ci.yml"
job_id = "rust"
step_name = "Test"
"#;

    let report = check_inventory_documents(&repository_root(), &contracts, gates);
    assert_eq!(report.declared_conforming_contracts, 1);
    assert_eq!(report.self_attested_contract_bindings_matched, 0);
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.contains("future trusted receipt adapter")),
        "{:#?}",
        report.violations
    );

    let without_claim = gates.replace(
        "evidence_claims = [{ contract_id = \"test-contract\", evidence_subject = \"baozi.test.contract\", evidence_revision = 1 }]\n",
        "",
    );
    assert_rejected(&contracts, &without_claim, "exact evidence claim");
}

#[test]
fn malformed_executor_and_unknown_fields_fail_closed() {
    let workflow_command = PARTIAL_GATES
        .replace("executor = \"command\"", "executor = \"workflow\"")
        .replace(
            "success_condition = \"process-exit-zero\"",
            "success_condition = \"required-workflow-status\"",
        );
    assert_rejected(PARTIAL_CONTRACTS, &workflow_command, "workflow gate");

    let ci_command = PARTIAL_GATES.replace("tracer_class = \"test\"", "tracer_class = \"ci\"");
    assert_rejected(PARTIAL_CONTRACTS, &ci_command, "command gate");

    let unknown_field = format!("{PARTIAL_CONTRACTS}\nunknown = true\n");
    assert_rejected(&unknown_field, PARTIAL_GATES, "failed to parse");
}

#[test]
fn repository_conformance_cli_checks_only_the_structured_inventory() {
    let output = Command::new(env!("CARGO_BIN_EXE_baozi-xtask"))
        .arg("repository-conformance-check")
        .env("CARGO", "repository-conformance-check-must-not-run-cargo")
        .current_dir(repository_root())
        .output()
        .expect("run repository conformance checker");

    assert_eq!(output.status.code(), Some(0), "{output:#?}");
    assert!(output.stderr.is_empty(), "{output:#?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("12 contracts, 13 gates"), "{output:#?}");
}
