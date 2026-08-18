use std::process::Command;

const SUPPORTED_COMMANDS: [&str; 4] = [
    "crate-graph-check",
    "native-fs-check",
    "repository-conformance-check",
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
    assert!(!stdout.contains("adr-check"), "{output:#?}");
    assert!(output.stderr.is_empty(), "{output:#?}");
}

#[test]
fn retired_commands_are_rejected_as_unknown() {
    for command in ["adr-check", "u4c-metrics"] {
        let output = Command::new(env!("CARGO_BIN_EXE_baozi-xtask"))
            .arg(command)
            .output()
            .expect("run removed baozi-xtask command");

        assert_eq!(output.status.code(), Some(2), "{output:#?}");
        assert!(output.stdout.is_empty(), "{output:#?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("usage: cargo run -p baozi-xtask"),
            "{output:#?}"
        );
        assert!(!stderr.contains(command), "{output:#?}");
    }
}
