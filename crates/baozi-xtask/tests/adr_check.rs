use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[test]
fn checked_in_fixture_and_cli_exit_contract_are_valid() {
    let manifest_directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let valid_fixture = manifest_directory.join("tests/fixtures/valid");
    let invalid_fixture = manifest_directory.join("tests/fixtures/invalid");

    let success = run_cli(&valid_fixture, &["adr-check"]);
    assert_eq!(success.status.code(), Some(0), "{success:#?}");
    assert!(
        String::from_utf8_lossy(&success.stdout).contains("ADR governance check passed (3 ADRs)"),
        "{success:#?}"
    );

    let governance_failure = run_cli(&invalid_fixture, &["adr-check"]);
    assert_eq!(
        governance_failure.status.code(),
        Some(1),
        "{governance_failure:#?}"
    );
    assert!(
        String::from_utf8_lossy(&governance_failure.stderr).contains("ADR governance check failed"),
        "{governance_failure:#?}"
    );

    let usage_failure = run_cli(&valid_fixture, &["invalid-command"]);
    assert_eq!(usage_failure.status.code(), Some(2), "{usage_failure:#?}");
    assert!(
        String::from_utf8_lossy(&usage_failure.stderr).contains("usage:"),
        "{usage_failure:#?}"
    );
}

#[test]
fn checked_in_repository_passes_cli_gate() {
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("xtask crate must be under the workspace root")
        .to_path_buf();

    let output = run_cli(&repository_root, &["adr-check"]);

    assert_eq!(output.status.code(), Some(0), "{output:#?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("ADR governance check passed (38 ADRs)"),
        "{output:#?}"
    );
}

fn run_cli(current_directory: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_baozi-xtask"))
        .args(arguments)
        .current_dir(current_directory)
        .output()
        .expect("run baozi-xtask")
}
