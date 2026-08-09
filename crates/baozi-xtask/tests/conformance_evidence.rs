use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use baozi_xtask::conformance::check_inventory_evidence_bindings;
use sha2::{Digest, Sha256};

const REPOSITORY_REVISION: &str = "0123456789abcdef0123456789abcdef01234567";

const CONFORMING_CONTRACTS: &str = r#"
schema_version = 1

[[contracts]]
contract_id = "test-contract"
authority_adr = "docs/adr/0003-core-scene-ir-and-material-model.md"
conformance_status = "conforming"
evidence_subject = "baozi.test.contract"
evidence_revision = 1
implementation_tracers = ["Cargo.toml"]
test_tracers = ["Cargo.lock"]
ci_tracers = [".github/workflows/ci.yml"]
gate_ids = ["implementation-gate", "test-gate", "ci-gate"]
"#;

const CONFORMING_GATES: &str = r#"
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
program = "cargo"
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

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("baozi-xtask must be below the repository crates directory")
        .to_path_buf()
}

fn sha256_hex(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn valid_bundle_for(contracts: &str, gates: &str) -> String {
    let contracts_sha256 = sha256_hex(contracts);
    let gates_sha256 = sha256_hex(gates);
    format!(
        r#"
schema_version = 1
trust_model = "self-attested"

[[gate_results]]
gate_id = "implementation-gate"
executor = "command"
success_condition = "process-exit-zero"
evidence_identity = "baozi.test.implementation/v1"
outcome = "passed"
repository_revision = "{REPOSITORY_REVISION}"
contracts_sha256 = "{contracts_sha256}"
gates_sha256 = "{gates_sha256}"

[[gate_results.claims]]
contract_id = "test-contract"
evidence_subject = "baozi.test.contract"
evidence_revision = 1

[[gate_results]]
gate_id = "test-gate"
executor = "command"
success_condition = "process-exit-zero"
evidence_identity = "baozi.test.gate/v1"
outcome = "passed"
repository_revision = "{REPOSITORY_REVISION}"
contracts_sha256 = "{contracts_sha256}"
gates_sha256 = "{gates_sha256}"

[[gate_results]]
gate_id = "ci-gate"
executor = "workflow"
success_condition = "required-workflow-status"
evidence_identity = "github-actions/ci/rust/test"
outcome = "passed"
repository_revision = "{REPOSITORY_REVISION}"
contracts_sha256 = "{contracts_sha256}"
gates_sha256 = "{gates_sha256}"
"#
    )
}

fn valid_bundle() -> String {
    valid_bundle_for(CONFORMING_CONTRACTS, CONFORMING_GATES)
}

fn assert_binding_consistent(bundle: &str) {
    let report = check_inventory_evidence_bindings(
        &repository_root(),
        CONFORMING_CONTRACTS,
        CONFORMING_GATES,
        bundle,
        REPOSITORY_REVISION,
    );
    assert!(report.violations.is_empty(), "{:#?}", report.violations);
    assert_eq!(report.declared_conforming_contracts, 1);
    assert_eq!(report.self_attested_contract_bindings_matched, 1);
}

fn assert_rejected(bundle: &str, expected: &str) {
    let report = check_inventory_evidence_bindings(
        &repository_root(),
        CONFORMING_CONTRACTS,
        CONFORMING_GATES,
        bundle,
        REPOSITORY_REVISION,
    );
    assert_eq!(report.self_attested_contract_bindings_matched, 0);
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
fn evidence_binding_cli_reports_no_promotion_authority() {
    let repository_root = repository_root();
    let relative_path = format!(
        "target/baozi-xtask-self-attested-evidence-{}.toml",
        std::process::id()
    );
    let evidence_path = repository_root.join(&relative_path);
    fs::create_dir_all(
        evidence_path
            .parent()
            .expect("the evidence fixture must have a parent"),
    )
    .expect("the target directory must be available");
    fs::write(
        &evidence_path,
        "schema_version = 1\ntrust_model = \"self-attested\"\ngate_results = []\n",
    )
    .expect("the evidence fixture must be writable");

    let output = Command::new(env!("CARGO_BIN_EXE_baozi-xtask"))
        .current_dir(&repository_root)
        .args([
            "repository-conformance-evidence-binding-check",
            "--evidence",
            &relative_path,
            "--repository-revision",
            REPOSITORY_REVISION,
        ])
        .output()
        .expect("the conformance CLI must run");
    fs::remove_file(&evidence_path).expect("the evidence fixture must be removable");

    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).expect("CLI stdout must be UTF-8");
    assert!(
        stdout.contains("does not authorize conformance promotion"),
        "{stdout}"
    );
}

#[test]
fn exact_self_attested_results_bind_one_declared_conforming_contract() {
    assert_binding_consistent(&valid_bundle());
}

#[test]
fn trusted_receipt_mode_is_reserved_for_a_future_adapter() {
    let bundle = valid_bundle().replacen(
        "trust_model = \"self-attested\"",
        "trust_model = \"trusted-receipt\"",
        1,
    );
    assert_rejected(&bundle, "trust_model must be self-attested");
}

#[test]
fn oversized_evidence_bundle_fails_before_parsing() {
    let oversized = "x".repeat((256 * 1024) + 1);
    assert_rejected(&oversized, "evidence bundle exceeds 262144 bytes");
}

#[test]
fn missing_or_failed_gate_results_fail_closed() {
    assert_rejected(
        "schema_version = 1\ntrust_model = \"self-attested\"\ngate_results = []\n",
        "missing passing result",
    );
    let failed = valid_bundle().replacen("outcome = \"passed\"", "outcome = \"failed\"", 1);
    assert_rejected(&failed, "did not pass");

    let failed_secondary = valid_bundle().replacen(
        "gate_id = \"test-gate\"\nexecutor = \"command\"\nsuccess_condition = \"process-exit-zero\"\nevidence_identity = \"baozi.test.gate/v1\"\noutcome = \"passed\"",
        "gate_id = \"test-gate\"\nexecutor = \"command\"\nsuccess_condition = \"process-exit-zero\"\nevidence_identity = \"baozi.test.gate/v1\"\noutcome = \"failed\"",
        1,
    );
    assert_rejected(&failed_secondary, "did not pass");
}

#[test]
fn unrelated_partial_contract_gates_do_not_require_results() {
    let contracts = format!(
        r#"{CONFORMING_CONTRACTS}

[[contracts]]
contract_id = "partial-contract"
authority_adr = "docs/adr/0003-core-scene-ir-and-material-model.md"
conformance_status = "partial"
evidence_subject = "baozi.test.partial"
evidence_revision = 1
implementation_tracers = ["Cargo.toml"]
test_tracers = ["Cargo.lock"]
ci_tracers = [".github/workflows/ci.yml"]
gate_ids = ["partial-gate"]
"#
    );
    let gates = format!(
        r#"{CONFORMING_GATES}

[[gates]]
gate_id = "partial-gate"
tracer_class = "test"
executor = "command"
success_condition = "process-exit-zero"
evidence_identity = "baozi.test.partial-gate/v1"
evidence_paths = ["Cargo.lock"]
applicable_contracts = ["partial-contract"]
program = "cargo"
args = ["test", "--locked"]
"#
    );
    let contracts_sha256 = sha256_hex(&contracts);
    let gates_sha256 = sha256_hex(&gates);
    let bundle = format!(
        r#"{}

[[gate_results]]
gate_id = "partial-gate"
executor = "command"
success_condition = "process-exit-zero"
evidence_identity = "baozi.test.partial-gate/v1"
outcome = "failed"
repository_revision = "{REPOSITORY_REVISION}"
contracts_sha256 = "{contracts_sha256}"
gates_sha256 = "{gates_sha256}"
"#,
        valid_bundle_for(&contracts, &gates)
    );
    let report = check_inventory_evidence_bindings(
        &repository_root(),
        &contracts,
        &gates,
        &bundle,
        REPOSITORY_REVISION,
    );

    assert!(report.violations.is_empty(), "{:#?}", report.violations);
    assert_eq!(report.declared_conforming_contracts, 1);
    assert_eq!(report.self_attested_contract_bindings_matched, 1);
}

#[test]
fn stale_revision_or_catalog_digest_fails_closed() {
    let stale_revision = valid_bundle().replacen(REPOSITORY_REVISION, &"f".repeat(40), 1);
    assert_rejected(&stale_revision, "repository revision");

    let actual = sha256_hex(CONFORMING_CONTRACTS);
    let stale_catalog = valid_bundle().replacen(&actual, &"0".repeat(64), 1);
    assert_rejected(&stale_catalog, "contract inventory digest");
}

#[test]
fn executor_identity_and_exact_claim_mismatches_fail_closed() {
    let wrong_executor =
        valid_bundle().replacen("executor = \"command\"", "executor = \"workflow\"", 1);
    assert_rejected(&wrong_executor, "executor");

    let wrong_identity = valid_bundle().replacen(
        "baozi.test.implementation/v1",
        "baozi.test.implementation/v2",
        1,
    );
    assert_rejected(&wrong_identity, "evidence identity");

    let wrong_claim = valid_bundle().replacen("evidence_revision = 1", "evidence_revision = 2", 1);
    assert_rejected(&wrong_claim, "evidence claim");
}

#[test]
fn duplicate_and_unknown_gate_results_fail_closed() {
    let bundle = valid_bundle();
    let first_result = bundle
        .split("[[gate_results]]")
        .nth(1)
        .expect("the bundle has an implementation result");
    let duplicate = format!("{bundle}\n[[gate_results]]{first_result}");
    assert_rejected(&duplicate, "duplicate gate result");

    let unknown = valid_bundle().replacen(
        "gate_id = \"implementation-gate\"",
        "gate_id = \"unknown-gate\"",
        1,
    );
    assert_rejected(&unknown, "unknown gate");
}
