use std::process::Command;

const SUPPORTED_COMMANDS: [&str; 3] = [
    "crate-graph-check",
    "native-fs-check",
    "runtime-admission-check",
];

#[test]
fn help_lists_only_supported_repository_checks() {
    let output = Command::new(env!("CARGO_BIN_EXE_baozi-xtask"))
        .arg("--help")
        .output()
        .expect("run baozi-xtask help");

    assert_eq!(output.status.code(), Some(0), "{output:#?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    for command in SUPPORTED_COMMANDS {
        assert!(stdout.contains(command), "{output:#?}");
    }
    assert!(!stdout.contains("repository-conformance"), "{output:#?}");
    assert!(!stdout.contains("adr-check"), "{output:#?}");
    assert!(output.stderr.is_empty(), "{output:#?}");
}

#[test]
fn retired_commands_are_rejected_as_unknown() {
    for command in [
        "adr-check",
        "u4c-metrics",
        "repository-conformance-check",
        "repository-conformance-evidence-binding-check",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_baozi-xtask"))
            .arg(command)
            .output()
            .expect("run removed baozi-xtask command");

        assert_eq!(output.status.code(), Some(2), "{output:#?}");
        assert!(output.stdout.is_empty(), "{output:#?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("usage: cargo run --locked -p baozi-xtask"),
            "{output:#?}"
        );
        assert!(!stderr.contains(command), "{output:#?}");
    }
}

#[test]
fn runtime_dependency_cli_checks_the_repository_from_a_nested_directory() {
    let output = Command::new(env!("CARGO_BIN_EXE_baozi-xtask"))
        .arg("runtime-admission-check")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run runtime dependency check");

    assert_eq!(output.status.code(), Some(0), "{output:#?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Runtime dependency check passed"),
        "{output:#?}"
    );
    assert!(output.stderr.is_empty(), "{output:#?}");
}

#[test]
fn receipt_options_and_retired_evidence_binding_are_rejected() {
    for arguments in [
        &["runtime-admission-check", "--receipt"][..],
        &[
            "repository-conformance-evidence-binding-check",
            "--evidence",
            "unused.json",
            "--repository-revision",
            "deadbeef",
        ][..],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_baozi-xtask"))
            .args(arguments)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("run baozi-xtask with retired receipt arguments");

        assert_eq!(output.status.code(), Some(2), "{output:#?}");
        assert!(output.stdout.is_empty(), "{output:#?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("usage:"),
            "{output:#?}"
        );
    }
}
