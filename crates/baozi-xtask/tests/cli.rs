use std::process::Command;

const SUPPORTED_COMMANDS: [&str; 4] = [
    "crate-graph-check",
    "native-fs-check",
    "runtime-admission-check",
    "u4c-metrics",
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
fn removed_adr_check_is_rejected_as_an_unknown_command() {
    let output = Command::new(env!("CARGO_BIN_EXE_baozi-xtask"))
        .arg("adr-check")
        .output()
        .expect("run removed baozi-xtask command");

    assert_eq!(output.status.code(), Some(2), "{output:#?}");
    assert!(output.stdout.is_empty(), "{output:#?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("usage: cargo run -p baozi-xtask"),
        "{output:#?}"
    );
    assert!(!stderr.contains("adr-check"), "{output:#?}");
}

#[test]
fn u4c_metrics_requires_an_explicit_specification_path() {
    let output = Command::new(env!("CARGO_BIN_EXE_baozi-xtask"))
        .arg("u4c-metrics")
        .output()
        .expect("run baozi-xtask without a u4c-metrics specification");

    assert_eq!(output.status.code(), Some(2), "{output:#?}");
    assert!(output.stdout.is_empty(), "{output:#?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("usage: cargo run -p baozi-xtask"),
        "{output:#?}"
    );
    assert!(stderr.contains("u4c-metrics --spec <path>"), "{output:#?}");
}

#[test]
fn u4c_metrics_rejects_unexpected_arguments_with_usage() {
    let output = Command::new(env!("CARGO_BIN_EXE_baozi-xtask"))
        .args(["u4c-metrics", "unexpected"])
        .output()
        .expect("run baozi-xtask with unexpected u4c-metrics argument");

    assert_eq!(output.status.code(), Some(2), "{output:#?}");
    assert!(output.stdout.is_empty(), "{output:#?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("usage: cargo run -p baozi-xtask"),
        "{output:#?}"
    );
    assert!(stderr.contains("u4c-metrics"), "{output:#?}");
}
