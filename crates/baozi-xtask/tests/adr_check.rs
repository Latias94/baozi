use std::fs;
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
    let adr_count = checked_in_adr_count(&repository_root);
    let expected = format!("ADR governance check passed ({adr_count} ADRs)");

    assert_eq!(output.status.code(), Some(0), "{output:#?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(&expected),
        "{output:#?}"
    );
}

fn checked_in_adr_count(repository_root: &Path) -> usize {
    fs::read_dir(repository_root.join("docs/adr"))
        .expect("read checked-in ADR directory")
        .map(|entry| entry.expect("read checked-in ADR entry"))
        .filter(|entry| {
            let file_name = entry.file_name();
            let Some(file_name) = file_name.to_str() else {
                return false;
            };
            let bytes = file_name.as_bytes();
            bytes.len() > 8
                && bytes[..4].iter().all(u8::is_ascii_digit)
                && bytes[4] == b'-'
                && bytes.ends_with(b".md")
        })
        .count()
}

fn run_cli(current_directory: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_baozi-xtask"))
        .args(arguments)
        .current_dir(current_directory)
        .output()
        .expect("run baozi-xtask")
}
