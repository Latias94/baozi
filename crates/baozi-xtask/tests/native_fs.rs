use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const SOURCE: &str = "registry+https://github.com/rust-lang/crates.io-index";
const CAP_STD_ID: &str = "registry+https://github.com/rust-lang/crates.io-index#cap-std@4.0.2";
const CAP_FS_EXT_ID: &str =
    "registry+https://github.com/rust-lang/crates.io-index#cap-fs-ext@4.0.2";
const CAP_PRIMITIVES_ID: &str =
    "registry+https://github.com/rust-lang/crates.io-index#cap-primitives@4.0.2";
const RUSTIX_ID: &str = "registry+https://github.com/rust-lang/crates.io-index#rustix@1.1.4";
const FIXTURE_IO_ID: &str = "path+file:///fixture/baozi-io#0.1.0";
const FIXTURE_FACADE_ID: &str = "path+file:///fixture/baozi#0.1.0";
const FIXTURE_XTASK_ID: &str = "path+file:///fixture/baozi-xtask#0.1.0";
const RUNTIME_CANDIDATE_TARGETS: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "aarch64-apple-darwin",
];
const DEPENDENCY_AUDIT_TARGETS: &[(&str, bool)] = &[
    ("x86_64-unknown-linux-gnu", true),
    ("x86_64-pc-windows-msvc", false),
    ("aarch64-apple-darwin", true),
    ("x86_64-unknown-freebsd", true),
];
const PORTABLE_TARGETS: &[&str] = &["wasm32-unknown-unknown", "wasm32-wasip1"];
const LOCKFILE: &str = r#"
version = 4

[[package]]
name = "cap-fs-ext"
version = "4.0.2"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "d78e5a3368ae89b7cb68186411452b4b9fac8b41be9c19bf3f47c2d2c8e36e6b"

[[package]]
name = "cap-primitives"
version = "4.0.2"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "cdadbd7c002d3a484b35243669abdae85a0ebaded5a61117169dc3400f9a7ff0"

[[package]]
name = "cap-std"
version = "4.0.2"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "7281235d6e96d3544ca18bba9049be92f4190f8d923e3caef1b5f66cfa752608"

[[package]]
name = "rustix"
version = "1.1.4"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "b6fe4565b9518b83ef4f91bb47ce29620ca828bd32cb7e408f0062e9930ba190"
"#;

#[test]
fn canonical_target_filtered_fixture_passes() {
    let fixture = Fixture::canonical();
    let report = fixture.check();

    assert!(report.violations.is_empty(), "{:#?}", report.violations);
    assert_eq!(report.profiles_checked, 15);
    assert_eq!(fixture.baseline["schema"], json!(3));
    assert_eq!(fixture.repository_baseline["schema"], json!(2));
    assert_eq!(
        fixture.baseline["runtime_candidate_targets"],
        json!(RUNTIME_CANDIDATE_TARGETS)
    );
    assert_eq!(
        fixture.repository_baseline["repository_operational_candidate_targets"],
        json!(["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"])
    );
    assert_eq!(
        fixture.repository_baseline["repository_fail_closed_targets"],
        json!(["x86_64-pc-windows-msvc"])
    );
    assert_eq!(fixture.baseline["release_registered_targets"], json!([]));
    assert_eq!(
        fixture.baseline["dependency_audit_profiles"]
            .as_object()
            .expect("dependency audit profiles")
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["freebsd", "linux", "macos", "windows-msvc"])
    );
    assert!(fixture.baseline.get("native_profiles").is_none());
}

#[test]
fn baseline_target_universes_are_shape_checked() {
    let mut fixture = Fixture::canonical();
    fixture.baseline["runtime_candidate_targets"] = json!([
        "x86_64-unknown-linux-gnu",
        "x86_64-pc-windows-msvc",
        "aarch64-apple-darwin",
        "x86_64-unknown-freebsd"
    ]);
    fixture.baseline["release_registered_targets"] = json!(["x86_64-apple-darwin"]);
    fixture.baseline["dependency_audit_profiles"]
        .as_object_mut()
        .expect("dependency audit profiles")
        .remove("freebsd");

    let report = fixture.check();

    for expected in [
        "runtime candidate targets must be",
        "release registered targets must be",
        "dependency audit profile set must be",
        "release registered targets must be a subset of runtime candidate targets",
        "runtime candidate targets must be a subset of dependency audit targets",
    ] {
        assert!(
            report
                .violations
                .iter()
                .any(|violation| violation.contains(expected)),
            "missing {expected:?}: {:#?}",
            report.violations
        );
    }
}

#[test]
fn baseline_target_universes_reject_duplicate_targets() {
    let mut fixture = Fixture::canonical();
    fixture.baseline["runtime_candidate_targets"]
        .as_array_mut()
        .expect("runtime candidate targets")
        .push(json!("x86_64-unknown-linux-gnu"));
    fixture.baseline["dependency_audit_profiles"]["freebsd"]["target"] =
        json!("x86_64-unknown-linux-gnu");

    let report = fixture.check();

    for expected in [
        "runtime candidate targets contain duplicate",
        "dependency audit profiles contain duplicate target",
    ] {
        assert!(
            report
                .violations
                .iter()
                .any(|violation| violation.contains(expected)),
            "missing {expected:?}: {:#?}",
            report.violations
        );
    }
}

#[test]
fn repository_target_qualification_keeps_windows_fail_closed() {
    let mut fixture = Fixture::canonical();
    fixture.repository_baseline["repository_operational_candidate_targets"] = json!([
        "x86_64-unknown-linux-gnu",
        "x86_64-pc-windows-msvc",
        "aarch64-apple-darwin"
    ]);
    fixture.repository_baseline["repository_fail_closed_targets"] = json!([]);

    let report = fixture.check();

    for expected in [
        "repository operational candidate targets must be",
        "repository fail-closed targets must be",
    ] {
        assert!(
            report
                .violations
                .iter()
                .any(|violation| violation.contains(expected)),
            "missing {expected:?}: {:#?}",
            report.violations
        );
    }
}

#[test]
fn candidate_filesystem_tests_cannot_skip_missing_fixtures() {
    let canonical = include_str!("../../baozi-io/src/fs.rs");
    let canonical_violations = baozi_xtask::native_fs::candidate_test_source_violations(canonical);
    assert!(canonical_violations.is_empty(), "{canonical_violations:#?}");

    for (case, hostile, expected) in [
        (
            "textual skip marker",
            format!("// SKIP fixture\n{canonical}"),
            "a textual skip marker is forbidden",
        ),
        (
            "environment fixture marker",
            format!("// BAOZI_TEST_REPARSE\n{canonical}"),
            "an environment-dependent external fixture is forbidden",
        ),
        (
            "rename result suppression",
            format!(
                "fn hostile_rename() {{ if fs::rename(&source, &moved).is_ok() {{}} }}\n{canonical}"
            ),
            "must not turn an fs::rename error into a passing test",
        ),
    ] {
        let violations = baozi_xtask::native_fs::candidate_test_source_violations(&hostile);
        assert!(
            violations
                .iter()
                .any(|violation| violation.contains(expected)),
            "{case}: missing {expected:?}: {violations:#?}"
        );
    }
}

#[test]
fn candidate_filesystem_tests_reject_ast_level_skip_paths() {
    let canonical = include_str!("../../baozi-io/src/fs.rs");
    let first_test_attribute =
        "    #[test]\n    fn opens_and_inspects_the_same_regular_file_handle() {";
    let cases = [
        (
            "direct ignore",
            canonical.replacen(
                first_test_attribute,
                "    #[ignore = \"external fixture\"]\n    #[test]\n    fn opens_and_inspects_the_same_regular_file_handle() {",
                1,
            ),
            "ignored test",
        ),
        (
            "nested cfg_attr ignore",
            canonical.replacen(
                first_test_attribute,
                "    #[cfg_attr(any(), cfg_attr(all(), ignore))]\n    #[test]\n    fn opens_and_inspects_the_same_regular_file_handle() {",
                1,
            ),
            "ignored test",
        ),
        (
            "should panic",
            canonical.replacen(
                first_test_attribute,
                "    #[should_panic]\n    #[test]\n    fn opens_and_inspects_the_same_regular_file_handle() {",
                1,
            ),
            "should-panic test",
        ),
        (
            "unit return",
            canonical.replacen(
                first_test_attribute,
                "    #[test]\n    fn opens_and_inspects_the_same_regular_file_handle() { return ();",
                1,
            ),
            "early-success return",
        ),
        (
            "bare return",
            canonical.replacen(
                first_test_attribute,
                "    #[test]\n    fn opens_and_inspects_the_same_regular_file_handle() { return;",
                1,
            ),
            "early-success return",
        ),
        (
            "result return",
            canonical.replacen(
                first_test_attribute,
                "    #[test]\n    fn opens_and_inspects_the_same_regular_file_handle() -> Result<(), ()> { return Ok(());",
                1,
            ),
            "early-success return",
        ),
        (
            "missing required test",
            canonical.replacen(
                "fn opens_and_inspects_the_same_regular_file_handle()",
                "fn renamed_required_test()",
                1,
            ),
            "missing required test",
        ),
        (
            "conditional test module",
            canonical.replacen(
                "#[cfg(test)]\nmod tests {",
                "#[cfg(any())]\nmod tests {",
                1,
            ),
            "unsupported ancestor conditional attribute",
        ),
        (
            "duplicate required test",
            format!(
                "{canonical}\n#[test]\nfn opens_and_inspects_the_same_regular_file_handle() {{}}"
            ),
            "defined more than once",
        ),
        (
            "required test hidden in const block",
            format!(
                "{}\nconst _: () = {{ #[test] fn opens_and_inspects_the_same_regular_file_handle() {{}} }};",
                canonical.replacen(
                    "fn opens_and_inspects_the_same_regular_file_handle()",
                    "fn renamed_required_test()",
                    1,
                )
            ),
            "missing required test opens_and_inspects_the_same_regular_file_handle",
        ),
        (
            "swapped platform cfg",
            canonical.replacen(
                "    #[cfg(windows)]\n    #[test]\n    fn windows_firewall_rejects_ads_device_and_namespace_forms_before_open() {",
                "    #[cfg(unix)]\n    #[test]\n    fn windows_firewall_rejects_ads_device_and_namespace_forms_before_open() {",
                1,
            ),
            "has platform Some(Unix), expected Some(Windows)",
        ),
        (
            "parse error",
            format!("{canonical}\nfn"),
            "failed to parse candidate source",
        ),
    ];

    for (case, hostile, expected) in cases {
        let violations = baozi_xtask::native_fs::candidate_test_source_violations(&hostile);
        assert!(
            violations
                .iter()
                .any(|violation| violation.contains(expected)),
            "{case}: missing {expected:?}: {violations:#?}"
        );
    }
}

#[test]
fn required_candidate_tests_cannot_be_empty_trivial_or_macro_hidden() {
    let canonical = include_str!("../../baozi-io/src/fs.rs");
    let signature = "fn opens_and_inspects_the_same_regular_file_handle() {";
    for (case, replacement, expected) in [
        (
            "empty",
            "fn opens_and_inspects_the_same_regular_file_handle() {}\n    fn ignored_original_body() {",
            "has an empty body",
        ),
        (
            "trivial assertion",
            "fn opens_and_inspects_the_same_regular_file_handle() { assert!(true); }\n    fn ignored_original_body() {",
            "must exercise at least one function or method call",
        ),
        (
            "unknown macro",
            "fn opens_and_inspects_the_same_regular_file_handle() { hidden_success!(); assert!(true); drop(()); }\n    fn ignored_original_body() {",
            "invokes unsupported macro hidden_success!",
        ),
        (
            "cfg-elided body",
            "fn opens_and_inspects_the_same_regular_file_handle() { #[cfg(any())] { drop(()); assert!(true); } }\n    fn ignored_original_body() {",
            "contains an unsupported body conditional attribute",
        ),
    ] {
        let hostile = canonical.replacen(signature, replacement, 1);
        let violations = baozi_xtask::native_fs::candidate_test_source_violations(&hostile);
        assert!(
            violations
                .iter()
                .any(|violation| violation.contains(expected)),
            "{case}: missing {expected:?}: {violations:#?}"
        );
    }
}

#[test]
fn required_candidate_test_module_is_bound_by_normalized_ast_digest() {
    let canonical = include_str!("../../baozi-io/src/fs.rs");
    let hostile = canonical.replacen(
        "fn opens_and_inspects_the_same_regular_file_handle() {",
        "fn opens_and_inspects_the_same_regular_file_handle() { assert!(String::new().is_empty()); }\n    fn ignored_original_body() {",
        1,
    );

    let violations = baozi_xtask::native_fs::candidate_test_source_violations(&hostile);

    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("normalized test-module digest must be")),
        "{violations:#?}"
    );
}

#[test]
fn candidate_test_macros_cannot_be_shadowed_from_unhashed_ancestor_scope() {
    let runtime = include_str!("../../baozi-io/src/fs.rs");
    let shadow = r#"
macro_rules! assert {
    ($($tokens:tt)*) => {{}};
}
macro_rules! assert_eq {
    ($($tokens:tt)*) => {{}};
}
macro_rules! assert_ne {
    ($($tokens:tt)*) => {{}};
}
macro_rules! eprintln {
    ($($tokens:tt)*) => {{}};
}
macro_rules! format {
    ($($tokens:tt)*) => {{}};
}
macro_rules! matches {
    ($($tokens:tt)*) => { true };
}
macro_rules! panic {
    ($($tokens:tt)*) => {{}};
}
macro_rules! vec {
    ($($tokens:tt)*) => { Vec::new() };
}
"#;
    let hostile_runtime = format!("{shadow}\n{runtime}");
    let canonical_violations = baozi_xtask::native_fs::candidate_test_source_violations(runtime);
    let hostile_violations =
        baozi_xtask::native_fs::candidate_test_source_violations(&hostile_runtime);

    for name in [
        "assert",
        "assert_eq",
        "assert_ne",
        "eprintln",
        "format",
        "matches",
        "panic",
        "vec",
    ] {
        assert!(
            hostile_violations
                .iter()
                .any(|violation| violation.contains(&format!(
                    "ancestor scope shadows candidate-test macro {name}!"
                ))),
            "missing shadow rejection for {name}: {hostile_violations:#?}"
        );
    }
    let canonical_digest = canonical_violations
        .iter()
        .filter(|violation| violation.contains("normalized test-module digest"))
        .collect::<Vec<_>>();
    let hostile_digest = hostile_violations
        .iter()
        .filter(|violation| violation.contains("normalized test-module digest"))
        .collect::<Vec<_>>();
    assert_eq!(
        hostile_digest, canonical_digest,
        "ancestor-only shadowing must leave the normalized module digest unchanged"
    );

    for hostile_binding in [
        format!("use hostile::assert_eq;\n{runtime}"),
        format!("use hostile::no_op as matches;\n{runtime}"),
        format!("use hostile::*;\n{runtime}"),
    ] {
        let violations = baozi_xtask::native_fs::candidate_test_source_violations(&hostile_binding);
        assert!(
            violations
                .iter()
                .any(|violation| violation.contains("ancestor")
                    && violation.contains("shadow candidate-test macro")),
            "{violations:#?}"
        );
    }
    for (hostile_source, expected) in [
        (
            format!("#[macro_use]\nmod hostile {{}}\n{runtime}"),
            "ancestor #[macro_use]",
        ),
        (
            format!("inject_candidate_macro!();\n{runtime}"),
            "ancestor macro invocation inject_candidate_macro!",
        ),
    ] {
        let violations = baozi_xtask::native_fs::candidate_test_source_violations(&hostile_source);
        assert!(
            violations
                .iter()
                .any(|violation| violation.contains(expected)),
            "missing {expected:?}: {violations:#?}"
        );
    }

    let exported_nested_macro = format!(
        "#[cfg(test)] mod helper {{ #[macro_export] macro_rules! assert {{ ($($tokens:tt)*) => {{}}; }} }}\n{runtime}"
    );
    let violations =
        baozi_xtask::native_fs::candidate_test_source_violations(&exported_nested_macro);
    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("exports a macro")),
        "nested macro exports must not escape the digest boundary: {violations:#?}"
    );

    let repository_tests = include_str!("../../baozi-io/src/repository/tests.rs");
    let canonical_repository = format!("#[cfg(test)]\nmod tests {{\n{repository_tests}\n}}\n");
    let hostile_repository = format!("{shadow}\n{canonical_repository}");
    let canonical_repository_violations =
        baozi_xtask::native_fs::repository_candidate_test_source_violations(&canonical_repository);
    let hostile_repository_violations =
        baozi_xtask::native_fs::repository_candidate_test_source_violations(&hostile_repository);
    assert!(
        hostile_repository_violations
            .iter()
            .any(|violation| violation
                .contains("ancestor scope shadows candidate-test macro matches!")),
        "{hostile_repository_violations:#?}"
    );
    assert_eq!(
        hostile_repository_violations
            .iter()
            .filter(|violation| violation.contains("normalized test-module digest"))
            .collect::<Vec<_>>(),
        canonical_repository_violations
            .iter()
            .filter(|violation| violation.contains("normalized test-module digest"))
            .collect::<Vec<_>>(),
        "repository ancestor shadowing must leave the normalized module digest unchanged"
    );
}

#[test]
fn repository_candidate_test_gate_is_recursive_and_separate_from_runtime_tests() {
    let runtime = include_str!("../../baozi-io/src/fs.rs");
    let runtime_violations = baozi_xtask::native_fs::candidate_test_source_violations(runtime);
    assert!(
        runtime_violations.is_empty(),
        "runtime tests must not inherit repository requirements: {runtime_violations:#?}"
    );

    let repository_violations =
        baozi_xtask::native_fs::repository_candidate_test_tree_violations(&repository_root());
    assert!(
        repository_violations.is_empty(),
        "{repository_violations:#?}"
    );

    let repository_tests = include_str!("../../baozi-io/src/repository/tests.rs");
    let hostile = format!(
        "#[cfg(test)]\nmod tests {{\n{}\n}}",
        repository_tests.replacen(
            "fn deterministic_capture_records_files_and_empty_directories() {",
            "fn deterministic_capture_records_files_and_empty_directories() { return;",
            1,
        )
    );
    let hostile_violations =
        baozi_xtask::native_fs::repository_candidate_test_source_violations(&hostile);
    assert!(
        hostile_violations.iter().any(|violation| violation.contains(
            "repository candidate tests: test deterministic_capture_records_files_and_empty_directories contains an early-success return"
        )),
        "{hostile_violations:#?}"
    );
}

#[test]
fn repository_candidate_surface_is_bound_to_the_exact_target_cfg() {
    let canonical = include_str!("../../baozi-io/src/lib.rs");
    assert!(baozi_xtask::native_fs::repository_candidate_surface_violations(canonical).is_empty());

    for (case, hostile, expected) in [
        (
            "target removed from cfg",
            canonical.replace(
                "target_os = \"windows\"",
                "target_os = \"windows-disabled\"",
            ),
            "unsupported exact target tuple",
        ),
        (
            "re-export no longer public",
            canonical.replacen("pub use repository::{", "pub(crate) use repository::{", 1),
            "expected exactly one repository public re-export",
        ),
        (
            "crate feature is cfg-elided",
            format!("#![cfg_attr(feature = \"repository-fs-candidate\", cfg(any()))]\n{canonical}"),
            "crate-level cfg_attr attribute is forbidden",
        ),
        (
            "module path is redirected",
            canonical.replacen(
                "mod repository;",
                "#[path = \"repository_stub.rs\"]\nmod repository;",
                1,
            ),
            "no path override",
        ),
        (
            "public seam is truncated",
            canonical.replacen("    RepositoryFile,", "", 1),
            "repository public re-export set must be",
        ),
    ] {
        let violations = baozi_xtask::native_fs::repository_candidate_surface_violations(&hostile);
        assert!(
            violations
                .iter()
                .any(|violation| violation.contains(expected)),
            "{case}: missing {expected:?}: {violations:#?}"
        );
    }
}

#[test]
fn runtime_candidate_surface_is_bound_to_the_exact_target_cfg() {
    let canonical = include_str!("../../baozi-io/src/lib.rs");
    assert!(baozi_xtask::native_fs::runtime_candidate_surface_violations(canonical).is_empty());

    for (case, hostile, expected) in [
        (
            "target removed from cfg",
            canonical.replace(
                "target_os = \"windows\"",
                "target_os = \"windows-disabled\"",
            ),
            "unsupported exact target tuple",
        ),
        (
            "re-export no longer public",
            canonical.replacen("pub use fs::{", "pub(crate) use fs::{", 1),
            "expected exactly one filesystem public re-export",
        ),
        (
            "crate feature is cfg-elided",
            format!("#![cfg_attr(feature = \"fs-candidate\", cfg(any()))]\n{canonical}"),
            "crate-level cfg_attr attribute is forbidden",
        ),
        (
            "module path is redirected",
            canonical.replacen("mod fs;", "#[path = \"fs_stub.rs\"]\nmod fs;", 1),
            "filesystem module must be",
        ),
        (
            "public seam is truncated",
            canonical.replacen(
                "FileSystemAssetIo, NativeDirectoryCapability",
                "FileSystemAssetIo",
                1,
            ),
            "filesystem public re-export set must be",
        ),
    ] {
        let violations = baozi_xtask::native_fs::runtime_candidate_surface_violations(&hostile);
        assert!(
            violations
                .iter()
                .any(|violation| violation.contains(expected)),
            "{case}: missing {expected:?}: {violations:#?}"
        );
    }
}

#[test]
fn format_and_import_production_sources_use_only_backend_io() {
    let report = baozi_xtask::native_fs::check_repository(&repository_root());

    assert!(report.violations.is_empty(), "{:#?}", report.violations);
}

#[test]
fn direct_std_filesystem_paths_and_aliases_are_rejected() {
    for (case, hostile) in [
        (
            "qualified function",
            r#"fn load() { let _ = std::fs::read("asset.bin"); }"#,
        ),
        (
            "raw standard-library identifier",
            r#"fn load() { let _ = r#std::fs::read("asset.bin"); }"#,
        ),
        (
            "raw filesystem identifier",
            r#"fn load() { let _ = std::r#fs::read("asset.bin"); }"#,
        ),
        (
            "module alias",
            r#"use std::fs as host_fs; fn load() { let _ = host_fs::read("asset.bin"); }"#,
        ),
        (
            "local module alias",
            r#"fn load() { use std::fs as host_fs; let _ = host_fs::read("asset.bin"); }"#,
        ),
        (
            "file alias",
            r#"use std::fs::File as HostFile; fn load() { let _ = HostFile::open("asset.bin"); }"#,
        ),
        (
            "local file alias",
            r#"fn outer() { fn load() { use std::fs::File as HostFile; let _ = HostFile::open("asset.bin"); } }"#,
        ),
        (
            "open-options alias",
            r#"use std::{fs::OpenOptions as HostOptions}; fn load() { let _ = HostOptions::new(); }"#,
        ),
        (
            "standard-library alias",
            r#"use std as host_std; fn load() { let _ = host_std::fs::read("asset.bin"); }"#,
        ),
        (
            "local standard-library alias",
            r#"fn load() { use std as host_std; let _ = host_std::fs::read("asset.bin"); }"#,
        ),
        (
            "raw standard-library alias",
            r#"use r#std as host_std; fn load() { let _ = host_std::fs::read("asset.bin"); }"#,
        ),
        (
            "raw filesystem import",
            r#"use std::r#fs; fn load() { let _ = fs::read("asset.bin"); }"#,
        ),
        (
            "forward local standard-library alias",
            r#"fn load() { let _ = host_std::fs::read("asset.bin"); use std as host_std; }"#,
        ),
        (
            "extern-crate alias",
            r#"extern crate std as host_std; fn load() { let _ = host_std::fs::read("asset.bin"); }"#,
        ),
        (
            "namespace-qualified standard-library re-export",
            r#"mod bridge { pub use std as host_std; } fn load() { let _ = bridge::host_std::fs::read("asset.bin"); }"#,
        ),
        (
            "qualified associated function",
            r#"fn load() { let _ = <std::fs::File>::open("asset.bin"); }"#,
        ),
        (
            "macro expansion",
            r#"macro_rules! load { () => { std::fs::read("asset.bin") }; }"#,
        ),
        (
            "parameterized macro filesystem root",
            r#"macro_rules! load { ($root:ident) => { $root::fs::read("asset.bin") }; } load!(std);"#,
        ),
        ("included Rust source", r#"include!("generated.rs");"#),
        ("raw included Rust source", r#"r#include!("generated.rs");"#),
        (
            "mixed test configuration",
            r#"#[cfg(any(test, feature = "fixture"))] fn load() { let _ = std::fs::read("asset.bin"); }"#,
        ),
        (
            "nested inline module",
            r#"mod importer { fn load() { use std::fs::OpenOptions as HostOptions; let _ = HostOptions::new(); } }"#,
        ),
        (
            "block-local inline module",
            r#"fn outer() { mod importer { fn load() { let _ = std::fs::read("asset.bin"); } } }"#,
        ),
        (
            "mixed configuration local import",
            r#"#[cfg(any(test, feature = "fixture"))] mod importer { fn load() { use std::fs::File; let _ = File::open("asset.bin"); } }"#,
        ),
        ("custom module path", r#"#[path = "host.rs"] mod importer;"#),
        (
            "conditional custom module path",
            r#"#[cfg_attr(unix, path = "host.rs")] mod importer;"#,
        ),
    ] {
        let violations = baozi_xtask::native_fs::production_source_text_violations(hostile);
        assert!(!violations.is_empty(), "{case}: {violations:#?}");
    }
}

#[test]
fn candidate_production_implementations_reject_evidence_only_cfgs() {
    for (case, source, expected) in [
        (
            "runtime test branch",
            r#"#[cfg(test)] fn open_candidate() { follow_symlink(); }"#,
            "evidence-only cfg condition test",
        ),
        (
            "runtime non-test branch",
            r#"#[cfg(not(test))] fn open_candidate() { follow_symlink(); }"#,
            "evidence-only cfg condition test",
        ),
        (
            "repository debug branch",
            r#"#[cfg(debug_assertions)] fn capture_repository() { follow_symlink(); }"#,
            "evidence-only cfg condition debug_assertions",
        ),
        (
            "repository miri branch",
            r#"#[cfg(any(unix, miri))] fn capture_repository() { follow_symlink(); }"#,
            "evidence-only cfg condition miri",
        ),
        (
            "conditional attribute",
            r#"#[cfg_attr(test, inline(always))] fn capture_repository() {}"#,
            "cfg_attr is forbidden",
        ),
        (
            "runtime cfg macro",
            r#"fn open_candidate() { if cfg!(test) { follow_symlink(); } }"#,
            "cfg! is forbidden",
        ),
        (
            "qualified runtime cfg macro",
            r#"fn open_candidate() { if ::core::cfg!(test) { follow_symlink(); } }"#,
            "cfg! is forbidden",
        ),
        (
            "aliased runtime cfg macro",
            r#"use core::cfg as candidate_cfg; fn open_candidate() { if candidate_cfg!(test) { follow_symlink(); } }"#,
            "not an admitted exact core builtin",
        ),
        (
            "repository debug cfg macro",
            r#"fn capture_repository() { if cfg!(debug_assertions) { follow_symlink(); } }"#,
            "cfg! is forbidden",
        ),
        (
            "repository miri cfg macro",
            r#"fn capture_repository() { if cfg!(miri) { follow_symlink(); } }"#,
            "cfg! is forbidden",
        ),
        (
            "macro hides evidence-only branch",
            r#"
macro_rules! choose_candidate {
    () => {
        if cfg!(test) {
            follow_symlink();
        }
    };
}
"#,
            "macro_rules! is forbidden",
        ),
        (
            "qualified include macro",
            r#"fn open_candidate() { ::core::include!("generated.rs"); }"#,
            "include! is forbidden",
        ),
        (
            "aliased include macro",
            r#"use core::include as candidate_include; fn open_candidate() { candidate_include!("generated.rs"); }"#,
            "not an admitted exact core builtin",
        ),
    ] {
        let violations = baozi_xtask::native_fs::candidate_production_source_violations(source);
        assert!(
            violations
                .iter()
                .any(|violation| violation.contains(expected)),
            "{case}: missing {expected:?}: {violations:#?}"
        );
    }

    let allowed = baozi_xtask::native_fs::candidate_production_source_violations(
        "#[cfg(test)] mod tests { #[test] fn candidate_contract() {} }",
    );
    assert!(
        allowed.is_empty(),
        "the dedicated candidate test module is not production implementation: {allowed:#?}"
    );

    let public_tests = baozi_xtask::native_fs::candidate_production_source_violations(
        "#[cfg(test)] pub mod tests { #[test] fn candidate_contract() {} }",
    );
    assert!(
        public_tests
            .iter()
            .any(|violation| violation.contains("evidence-only cfg condition test")),
        "a public tests module is not the dedicated private test module: {public_tests:#?}"
    );
}

#[test]
fn source_injection_macros_are_rejected_without_expansion() {
    for (case, hostile) in [
        (
            "indirect include macro",
            r#"passthrough!(include!("generated.rs"));"#,
        ),
        (
            "qualified include macro",
            r#"::core::include!("generated.rs");"#,
        ),
        (
            "aliased include macro",
            r#"use core::include as injected_source; injected_source!("generated.rs");"#,
        ),
        (
            "macro-generated standard-library alias",
            r#"
macro_rules! inject_alias { () => { use std as host_std; } }
inject_alias!();
fn load() { let _ = host_std::fs::read("asset.bin"); }
"#,
        ),
        (
            "unsafe local macro shadows verified parent macro",
            r#"
macro_rules! define_item { () => { struct Marker; } }
mod nested {
    macro_rules! define_item { () => { external_source!(); } }
    define_item!();
}
"#,
        ),
    ] {
        let violations = baozi_xtask::native_fs::production_source_text_violations(hostile);
        assert!(!violations.is_empty(), "{case}: {violations:#?}");
    }
}

#[test]
fn renamed_std_namespace_is_rejected_without_filesystem_use() {
    let violations = baozi_xtask::native_fs::production_source_text_violations(
        "use std as host_std; mod parser {}",
    );

    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("renamed std namespace host_std")),
        "{violations:#?}"
    );
}

#[test]
fn source_policy_scans_custom_build_targets() {
    let workspace = SourcePolicyWorkspace::new("custom-build");
    workspace.write_package(
        "crates/baozi-format-build",
        "baozi-format-build",
        "build = \"build.rs\"",
        "pub fn parse() {}",
    );
    workspace.write_source(
        "crates/baozi-format-build/build.rs",
        r#"fn main() { let _ = std::fs::read("schema.bin"); }"#,
    );
    workspace.write_facade(
        "baozi-format-build = { path = \"../baozi-format-build\", optional = true }",
        "",
        "\"dep:baozi-format-build\"",
    );

    let violations = workspace.source_policy_violations();

    assert!(
        violations.iter().any(|violation| {
            violation.contains("direct host filesystem") && violation.contains("build.rs")
        }),
        "{violations:#?}"
    );
}

#[test]
fn source_policy_scans_facade_reachable_excluded_path_formats() {
    let workspace = SourcePolicyWorkspace::new("excluded-path-format");
    workspace.write_package(
        "vendor/baozi-format-excluded",
        "baozi-format-excluded",
        "",
        r#"pub fn parse() { let _ = std::fs::read("asset.bin"); }"#,
    );
    workspace.write_facade(
        "baozi-format-excluded = { path = \"../../vendor/baozi-format-excluded\", optional = true }",
        "",
        "\"dep:baozi-format-excluded\"",
    );

    let violations = workspace.source_policy_violations();

    assert!(
        violations.iter().any(|violation| {
            violation.contains("direct host filesystem")
                && violation.contains("baozi-format-excluded")
        }),
        "{violations:#?}"
    );
}

#[test]
fn source_policy_excludes_facade_dev_only_formats() {
    let workspace = SourcePolicyWorkspace::new("dev-only-format");
    workspace.write_package(
        "crates/baozi-format-clean",
        "baozi-format-clean",
        "",
        "pub fn parse() {}",
    );
    workspace.write_package(
        "crates/baozi-format-dev-only",
        "baozi-format-dev-only",
        "",
        r#"pub fn fixture() { let _ = std::fs::read("fixture.bin"); }"#,
    );
    workspace.write_facade(
        "baozi-format-clean = { path = \"../baozi-format-clean\", optional = true }",
        "baozi-format-dev-only = { path = \"../baozi-format-dev-only\" }",
        "\"dep:baozi-format-clean\"",
    );

    let violations = workspace.source_policy_violations();

    assert!(violations.is_empty(), "{violations:#?}");
}

#[test]
fn backend_io_non_filesystem_std_and_test_only_code_are_allowed() {
    let source = r#"
use std::io::{Read, Seek};
use baozi_io::AssetIo;

const POLICY_NOTE: &str = "std::fs::read is forbidden in production";
const MAGIC: &[u8] = include_bytes!("magic.bin");

macro_rules! define_marker {
    () => { struct Marker; }
}
define_marker!();

fn read_header(reader: &mut impl Read) {
    let _ = reader.read(&mut []);
    trace_value!(reader);
}

fn local_test_helpers_are_not_production() {
    #[cfg(test)]
    use std::fs::File;

    #[cfg(test)]
    fn fixture() {
        let _ = File::open("fixture.bin");
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{File, OpenOptions};

    fn fixture() {
        let _ = File::open("fixture.bin");
        let _ = OpenOptions::new();
    }
}
"#;

    let violations = baozi_xtask::native_fs::production_source_text_violations(source);
    assert!(violations.is_empty(), "{violations:#?}");
}

#[test]
fn dependency_target_predicates_must_bind_empty_target_abi() {
    let mut fixture = Fixture::canonical();
    for dependency in fixture.manifest_dependencies_mut() {
        let Some(target) = dependency.get("target").and_then(Value::as_str) else {
            continue;
        };
        let widened = target.replace(", target_abi = \"\"", "");
        dependency["target"] = Value::String(widened);
    }

    let report = fixture.check();

    for dependency in ["cap-std", "cap-fs-ext", "rustix"] {
        assert!(
            report
                .violations
                .iter()
                .any(|violation| violation.contains(&format!("direct {dependency} edge drifted"))),
            "ABI-widened {dependency} predicate was accepted: {:#?}",
            report.violations
        );
    }
}

#[test]
fn direct_hidden_backend_edges_are_rejected() {
    let mut fixture = Fixture::canonical();
    fixture
        .manifest_dependencies_mut()
        .push(json!({ "name": "cap-primitives" }));

    let report = fixture.check();

    assert!(
        report.violations.iter().any(|violation| violation
            == "native-fs manifest: baozi-io must not depend directly on cap-primitives"),
        "{:#?}",
        report.violations
    );
}

#[test]
fn native_backend_packages_are_private_to_baozi_io() {
    for dependency in ["cap-std", "cap-primitives", "rustix", "cap-fs-ext"] {
        let mut fixture = Fixture::canonical();
        fixture.manifest["packages"]
            .as_array_mut()
            .expect("fixture manifest packages")
            .push(json!({
                "name": "baozi-import",
                "features": {},
                "dependencies": [{ "name": dependency }]
            }));

        let report = fixture.check();

        assert!(
            report.violations.iter().any(|violation| violation
                == &format!(
                    "native-fs manifest: baozi-import must not depend directly on {dependency}; native filesystem dependencies are private to baozi-io"
                )),
            "{:#?}",
            report.violations
        );
    }
}

#[test]
fn facade_candidate_feature_must_delegate_to_baozi_io() {
    let mut fixture = Fixture::canonical();
    fixture.manifest["packages"][1]["features"]["native-fs-candidate"] = json!(["std"]);

    let report = fixture.check();

    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.contains("baozi/native-fs-candidate must be exactly")),
        "{:#?}",
        report.violations
    );
}

#[test]
fn repository_feature_and_xtask_edge_are_exact() {
    let mut missing_dependency = Fixture::canonical();
    missing_dependency
        .manifest_dependencies_mut()
        .retain(|dependency| dependency["name"] != "cap-fs-ext");
    let missing_dependency_report = missing_dependency.check();
    assert!(
        missing_dependency_report
            .violations
            .iter()
            .any(|violation| violation
                == "native-fs manifest: expected exactly one direct cap-fs-ext edge, found 0"),
        "{:#?}",
        missing_dependency_report.violations
    );

    let mut missing_feature = Fixture::canonical();
    missing_feature.manifest["packages"][0]["features"]["repository-fs-candidate"] =
        json!(["fs-candidate"]);
    let missing_feature_report = missing_feature.check();
    assert!(
        missing_feature_report.violations.iter().any(|violation| violation
            .contains("baozi-io/repository-fs-candidate must be exactly")),
        "{:#?}",
        missing_feature_report.violations
    );

    let mut xtask_default_features = Fixture::canonical();
    xtask_default_features.manifest["packages"][2]["dependencies"][0]["uses_default_features"] =
        json!(true);
    let xtask_report = xtask_default_features.check();
    assert!(
        xtask_report
            .violations
            .iter()
            .any(|violation| violation
                .contains("baozi-xtask direct baozi-io repository edge drifted")),
        "{:#?}",
        xtask_report.violations
    );

    let mut xtask_non_optional = Fixture::canonical();
    xtask_non_optional.manifest["packages"][2]["dependencies"][0]["optional"] = json!(false);
    let xtask_report = xtask_non_optional.check();
    assert!(
        xtask_report
            .violations
            .iter()
            .any(|violation| violation
                .contains("baozi-xtask direct baozi-io repository edge drifted")),
        "{:#?}",
        xtask_report.violations
    );

    let mut missing_xtask_feature = Fixture::canonical();
    missing_xtask_feature.manifest["packages"][2]["features"]["repository-input-candidate"] =
        json!([]);
    let xtask_report = missing_xtask_feature.check();
    assert!(
        xtask_report.violations.iter().any(|violation| violation
            .contains("baozi-xtask/repository-input-candidate must be exactly")),
        "{:#?}",
        xtask_report.violations
    );
}

#[test]
fn xtask_and_other_crates_cannot_own_native_backend_dependencies() {
    let mut xtask_fixture = Fixture::canonical();
    xtask_fixture.manifest["packages"][2]["dependencies"]
        .as_array_mut()
        .expect("xtask dependencies")
        .push(json!({ "name": "cap-std" }));
    let xtask_report = xtask_fixture.check();
    assert!(
        xtask_report.violations.iter().any(|violation| violation
            == "native-fs manifest: baozi-xtask must not depend directly on cap-std; native filesystem dependencies are private to baozi-io"),
        "{:#?}",
        xtask_report.violations
    );

    let mut other_fixture = Fixture::canonical();
    other_fixture.manifest["packages"]
        .as_array_mut()
        .expect("fixture manifest packages")
        .push(json!({
            "name": "baozi-core",
            "features": {},
            "dependencies": [{ "name": "cap-fs-ext" }]
        }));
    let other_report = other_fixture.check();
    assert!(
        other_report.violations.iter().any(|violation| violation
            == "native-fs manifest: baozi-core must not depend directly on cap-fs-ext; native filesystem dependencies are private to baozi-io"),
        "{:#?}",
        other_report.violations
    );
}

#[test]
fn only_the_registered_xtask_edge_can_request_repository_io() {
    let mut direct = Fixture::canonical();
    direct.manifest["packages"]
        .as_array_mut()
        .expect("fixture manifest packages")
        .push(json!({
            "name": "baozi-format-gltf",
            "features": {},
            "dependencies": [{
                "name": "baozi-io",
                "kind": "dev",
                "rename": null,
                "features": ["repository-fs-candidate"]
            }]
        }));
    let report = direct.check();
    assert!(
        report.violations.iter().any(|violation| violation
            == "native-fs manifest: baozi-format-gltf must not request baozi-io/repository-fs-candidate on a dependency edge"),
        "{:#?}",
        report.violations
    );

    let mut forwarded = Fixture::canonical();
    forwarded.manifest["packages"]
        .as_array_mut()
        .expect("fixture manifest packages")
        .push(json!({
            "name": "baozi-test-support",
            "features": {
                "repository": ["asset_io/repository-fs-candidate"]
            },
            "dependencies": [{
                "name": "baozi-io",
                "kind": "dev",
                "rename": "asset_io",
                "features": []
            }]
        }));
    let report = forwarded.check();
    assert!(
        report.violations.iter().any(|violation| violation
            == "native-fs manifest: baozi-test-support/repository must not forward baozi-io/repository-fs-candidate"),
        "{:#?}",
        report.violations
    );
}

#[test]
fn complete_local_declarations_reject_excluded_dev_helper_forwarding() {
    const GLTF_ID: &str = "path+file:///fixture/baozi-format-gltf#0.1.0";
    const HELPER_ID: &str = "path+file:///fixture/repository-leak-helper#0.1.0";
    let mut fixture = Fixture::canonical();
    fixture.local_declaration_metadata["packages"]
        .as_array_mut()
        .expect("complete declaration packages")
        .extend([
            json!({
                "id": GLTF_ID,
                "name": "baozi-format-gltf",
                "version": "0.1.0",
                "source": null,
                "manifest_path": "/fixture/crates/baozi-format-gltf/Cargo.toml",
                "features": {
                    "default": []
                },
                "dependencies": [{
                    "name": "repository-leak-helper",
                    "source": null,
                    "req": "*",
                    "kind": "dev",
                    "rename": "helper",
                    "optional": false,
                    "uses_default_features": false,
                    "features": ["leak"],
                    "target": null
                }]
            }),
            json!({
                "id": HELPER_ID,
                "name": "repository-leak-helper",
                "version": "0.1.0",
                "source": null,
                "manifest_path": "/fixture/vendor/repository-leak-helper/Cargo.toml",
                "features": {
                    "default": [],
                    "leak": ["nested"],
                    "nested": ["asset_io?/repository-fs-candidate"]
                },
                "dependencies": [{
                    "name": "baozi-io",
                    "source": null,
                    "req": "*",
                    "kind": null,
                    "rename": "asset_io",
                    "optional": true,
                    "uses_default_features": false,
                    "features": [],
                    "target": null
                }]
            }),
        ]);
    fixture.local_declaration_metadata["workspace_members"]
        .as_array_mut()
        .expect("workspace members")
        .push(json!(GLTF_ID));

    let report = fixture.check();

    assert!(
        report.violations.iter().any(|violation| violation
            == "native-fs manifest: repository-leak-helper/nested must not forward baozi-io/repository-fs-candidate"),
        "{:#?}",
        report.violations
    );
}

#[test]
fn complete_local_declarations_scan_build_and_target_specific_edges() {
    const BUILD_ID: &str = "path+file:///fixture/baozi-format-build#0.1.0";
    let mut fixture = Fixture::canonical();
    fixture.local_declaration_metadata["packages"]
        .as_array_mut()
        .expect("complete declaration packages")
        .push(json!({
            "id": BUILD_ID,
            "name": "baozi-format-build",
            "version": "0.1.0",
            "source": null,
            "manifest_path": "/fixture/crates/baozi-format-build/Cargo.toml",
            "features": {
                "default": []
            },
            "dependencies": [{
                "name": "baozi-io",
                "source": null,
                "req": "*",
                "kind": "build",
                "rename": null,
                "optional": true,
                "uses_default_features": false,
                "features": ["repository-fs-candidate"],
                "target": "cfg(target_os = \"windows\")"
            }]
        }));
    fixture.local_declaration_metadata["workspace_members"]
        .as_array_mut()
        .expect("workspace members")
        .push(json!(BUILD_ID));

    let report = fixture.check();

    assert!(
        report.violations.iter().any(|violation| violation
            == "native-fs manifest: baozi-format-build must not request baozi-io/repository-fs-candidate on a dependency edge"),
        "{:#?}",
        report.violations
    );
}

#[test]
fn complete_local_declarations_reject_same_name_xtask_spoofs() {
    const SPOOF_ID: &str = "path+file:///fixture/spoofed-baozi-xtask#0.1.0";
    let mut fixture = Fixture::canonical();
    fixture.local_declaration_metadata["packages"]
        .as_array_mut()
        .expect("complete declaration packages")
        .push(json!({
            "id": SPOOF_ID,
            "name": "baozi-xtask",
            "version": "0.1.0",
            "source": null,
            "manifest_path": "/fixture/vendor/baozi-xtask/Cargo.toml",
            "features": {
                "default": []
            },
            "dependencies": []
        }));

    let report = fixture.check();

    assert!(
        report.violations.iter().any(|violation| {
            violation.contains("native-fs complete local declarations")
                && violation.contains("manifest_path must be")
        }),
        "{:#?}",
        report.violations
    );
}

#[test]
fn facade_candidate_cannot_reach_repository_feature() {
    let mut fixture = Fixture::canonical();
    fixture.manifest["packages"][1]["features"]["std"] =
        json!(["baozi-io/repository-fs-candidate"]);

    let report = fixture.check();

    assert!(
        report.violations.iter().any(|violation| violation
            == "native-fs manifest: baozi/native-fs-candidate must not reach baozi-io/repository-fs-candidate"),
        "{:#?}",
        report.violations
    );
}

#[test]
fn resolved_facade_candidate_rejects_indirect_repository_activation() {
    let mut fixture = Fixture::canonical();
    let metadata = fixture
        .facade_candidate_target_metadata
        .get_mut("x86_64-unknown-linux-gnu")
        .expect("linux facade candidate profile");
    add_cap_fs_ext(metadata, true);

    let report = fixture.check();

    assert!(
        report.violations.iter().any(|violation| violation.contains(
            "facade candidate profile x86_64-unknown-linux-gnu: baozi-io resolved features must be exactly"
        )),
        "{:#?}",
        report.violations
    );
}

#[test]
fn resolved_facade_candidate_binds_workspace_baozi_io_identity() {
    let mut fixture = Fixture::canonical();
    fixture
        .facade_candidate_target_metadata
        .get_mut("x86_64-unknown-linux-gnu")
        .expect("linux facade candidate profile")["packages"]
        .as_array_mut()
        .expect("fixture packages")
        .iter_mut()
        .find(|package| package["name"] == "baozi-io")
        .expect("baozi-io package")["manifest_path"] = json!("/fixture/vendor/baozi-io/Cargo.toml");

    let report = fixture.check();

    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.contains("manifest_path must be")),
        "{:#?}",
        report.violations
    );
}

#[test]
fn xtask_repository_edge_has_one_feature_entry_and_workspace_identity() {
    let mut alternate = Fixture::canonical();
    alternate.manifest["packages"][2]["features"]["alternate"] = json!(["dep:baozi-io"]);
    let report = alternate.check();
    assert!(
        report.violations.iter().any(|violation| violation.contains(
            "baozi-xtask/alternate must not provide an alternate activation path to baozi-io"
        )),
        "{:#?}",
        report.violations
    );

    let mut external_path = Fixture::canonical();
    external_path.xtask_all_metadata["packages"]
        .as_array_mut()
        .expect("fixture packages")
        .iter_mut()
        .find(|package| package["name"] == "baozi-io")
        .expect("baozi-io package")["manifest_path"] = json!("/fixture/vendor/baozi-io/Cargo.toml");
    let report = external_path.check();
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.contains("manifest_path must be")),
        "{:#?}",
        report.violations
    );
}

#[test]
fn xtask_resolved_profiles_reject_indirect_and_default_repository_paths() {
    const HELPER_ID: &str = "path+file:///fixture/xtask-helper#0.1.0";
    const IO_ID: &str = "path+file:///fixture/baozi-io#0.1.0";
    const XTASK_ID: &str = "path+file:///fixture/baozi-xtask#0.1.0";

    let mut all_features = Fixture::canonical();
    all_features.manifest["packages"][2]["features"]["alternate"] = json!(["dep:helper"]);
    let metadata = &mut all_features.xtask_all_metadata;
    let mut helper = package(HELPER_ID, "xtask-helper", "0.1.0");
    helper["manifest_path"] = json!("/fixture/crates/xtask-helper/Cargo.toml");
    metadata["packages"]
        .as_array_mut()
        .expect("fixture packages")
        .push(helper);
    let nodes = metadata["resolve"]["nodes"]
        .as_array_mut()
        .expect("fixture nodes");
    let root = nodes
        .iter_mut()
        .find(|node| node["id"] == XTASK_ID)
        .expect("fixture xtask node");
    root["features"] = json!(["alternate", "repository-input-candidate"]);
    root["deps"]
        .as_array_mut()
        .expect("fixture xtask deps")
        .push(dependency("helper", HELPER_ID));
    nodes.push(node(HELPER_ID, vec![dependency("baozi_io", IO_ID)], vec![]));
    metadata["workspace_members"]
        .as_array_mut()
        .expect("workspace members")
        .push(json!(HELPER_ID));
    let report = all_features.check();
    assert!(
        report.violations.iter().any(|violation| violation.contains(
            "baozi-xtask resolved features must be exactly [\"repository-input-candidate\"]"
        )),
        "{:#?}",
        report.violations
    );

    let mut default_path = Fixture::canonical();
    let metadata = &mut default_path.xtask_empty_metadata;
    let mut helper = package(HELPER_ID, "xtask-helper", "0.1.0");
    helper["manifest_path"] = json!("/fixture/crates/xtask-helper/Cargo.toml");
    metadata["packages"]
        .as_array_mut()
        .expect("fixture packages")
        .push(helper);
    let nodes = metadata["resolve"]["nodes"]
        .as_array_mut()
        .expect("fixture nodes");
    nodes
        .iter_mut()
        .find(|node| node["id"] == XTASK_ID)
        .expect("fixture xtask node")["deps"] = json!([dependency("helper", HELPER_ID)]);
    nodes.push(node(HELPER_ID, vec![dependency("baozi_io", IO_ID)], vec![]));
    metadata["workspace_members"]
        .as_array_mut()
        .expect("workspace members")
        .push(json!(HELPER_ID));
    let report = default_path.check();
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.contains("empty-feature xtask must not reach baozi-io")),
        "{:#?}",
        report.violations
    );
}

#[test]
fn release_native_fs_features_must_remain_unregistered() {
    let mut io_fixture = Fixture::canonical();
    io_fixture.manifest["packages"][0]["features"]["fs"] = json!([]);
    let io_report = io_fixture.check();
    assert!(
        io_report
            .violations
            .iter()
            .any(|violation| violation.contains("baozi-io/fs must remain unregistered")),
        "{:#?}",
        io_report.violations
    );

    let mut facade_fixture = Fixture::canonical();
    facade_fixture.manifest["packages"][1]["features"]["native-fs"] =
        json!(["std", "baozi-io/fs-candidate"]);
    let facade_report = facade_fixture.check();
    assert!(
        facade_report
            .violations
            .iter()
            .any(|violation| violation.contains("baozi/native-fs must remain unregistered")),
        "{:#?}",
        facade_report.violations
    );
}

#[test]
fn default_features_must_not_enable_the_native_adapter() {
    let mut fixture = Fixture::canonical();
    fixture.manifest["packages"][0]["features"]["default"] = json!(["fs-candidate"]);

    let report = fixture.check();

    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.contains("baozi-io/default must be empty")),
        "{:#?}",
        report.violations
    );
}

#[test]
fn facade_portable_feature_sets_must_not_reach_native_fs() {
    let mut fixture = Fixture::canonical();
    fixture.manifest["packages"][1]["features"]["portable-helper"] = json!(["native-fs-candidate"]);
    fixture.manifest["packages"][1]["features"]["all-formats"] = json!(["portable-helper"]);

    let report = fixture.check();

    assert!(
        report.violations.iter().any(|violation| violation
            .contains("baozi/all-formats must not enable the native filesystem adapter")),
        "{:#?}",
        report.violations
    );

    let mut weak_dependency_fixture = Fixture::canonical();
    weak_dependency_fixture.manifest["packages"][1]["features"]["all-formats"] =
        json!(["baozi-io?/fs-candidate"]);
    let weak_dependency_report = weak_dependency_fixture.check();
    assert!(
        weak_dependency_report
            .violations
            .iter()
            .any(|violation| violation
                .contains("baozi/all-formats must not enable the native filesystem adapter")),
        "{:#?}",
        weak_dependency_report.violations
    );
}

#[test]
fn checksum_and_resolved_feature_drift_are_rejected() {
    let canonical = Fixture::canonical();
    let baseline = canonical.baseline.clone();

    let mut checksum_drift = Fixture::canonical();
    checksum_drift.baseline = baseline.clone();
    checksum_drift.lockfile = checksum_drift.lockfile.replace(
        "b6fe4565b9518b83ef4f91bb47ce29620ca828bd32cb7e408f0062e9930ba190",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    let checksum_report = checksum_drift.check();
    assert!(
        checksum_report.violations.iter().any(|violation| violation
            .contains("rustix must resolve to 1.1.4")
            || violation.contains("rustix@1.1.4.checksum")),
        "{:#?}",
        checksum_report.violations
    );

    let mut feature_drift = Fixture::canonical();
    feature_drift.baseline = baseline;
    feature_drift
        .target_metadata
        .get_mut("x86_64-unknown-linux-gnu")
        .expect("fixture Linux metadata")["resolve"]["nodes"]
        .as_array_mut()
        .expect("fixture nodes")
        .iter_mut()
        .find(|node| node["id"] == RUSTIX_ID)
        .expect("fixture rustix node")["features"] = json!(["fs", "net", "std"]);
    let feature_report = feature_drift.check();
    assert!(
        feature_report
            .violations
            .iter()
            .any(|violation| violation.contains("rustix@1.1.4.features")),
        "{:#?}",
        feature_report.violations
    );
}

#[test]
fn runtime_candidate_closure_must_not_reach_cap_fs_ext() {
    let mut fixture = Fixture::canonical();
    let linux = fixture
        .target_metadata
        .get_mut("x86_64-unknown-linux-gnu")
        .expect("fixture Linux metadata");
    add_cap_fs_ext(linux, false);

    let report = fixture.check();

    assert!(
        report.violations.iter().any(|violation| violation
            == "native-fs profile linux: cap-fs-ext is forbidden in the resolved closure"),
        "{:#?}",
        report.violations
    );
}

#[test]
fn native_packages_cannot_hide_behind_a_sibling_helper_dependency() {
    const HELPER_ID: &str = "path+file:///fixture/native-helper#0.1.0";
    const SECOND_CAP_STD_ID: &str =
        "registry+https://github.com/rust-lang/crates.io-index#cap-std@4.0.1";
    let mut fixture = Fixture::canonical();
    let linux = fixture
        .target_metadata
        .get_mut("x86_64-unknown-linux-gnu")
        .expect("fixture Linux metadata");
    linux["packages"]
        .as_array_mut()
        .expect("fixture packages")
        .extend([
            package(HELPER_ID, "native-helper", "0.1.0"),
            package(SECOND_CAP_STD_ID, "cap-std", "4.0.1"),
        ]);
    let nodes = linux["resolve"]["nodes"]
        .as_array_mut()
        .expect("fixture nodes");
    nodes
        .iter_mut()
        .find(|node| node["id"] == "path+file:///fixture/baozi-io#0.1.0")
        .expect("fixture root node")["deps"]
        .as_array_mut()
        .expect("fixture root dependencies")
        .push(dependency("native_helper", HELPER_ID));
    nodes.extend([
        node(
            HELPER_ID,
            vec![dependency("cap_std", SECOND_CAP_STD_ID)],
            vec![],
        ),
        node(SECOND_CAP_STD_ID, vec![], vec![]),
    ]);

    let report = fixture.check();

    assert!(
        report.violations.iter().any(|violation| violation.contains(
            "native package cap-std is reachable outside the admitted direct native dependency subgraph"
        )),
        "{:#?}",
        report.violations
    );
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.contains("expected exactly one cap-std package, found 2")),
        "{:#?}",
        report.violations
    );
}

#[test]
fn repository_closure_feature_checksum_and_baseline_drift_are_rejected() {
    let mut missing_dependency = Fixture::canonical();
    let missing_linux = missing_dependency
        .repository_target_metadata
        .get_mut("x86_64-unknown-linux-gnu")
        .expect("fixture repository Linux metadata");
    missing_linux["resolve"]["nodes"]
        .as_array_mut()
        .expect("fixture nodes")
        .iter_mut()
        .find(|node| node["id"] == "path+file:///fixture/baozi-io#0.1.0")
        .expect("fixture root node")["deps"]
        .as_array_mut()
        .expect("fixture root dependencies")
        .retain(|dependency| dependency["pkg"] != CAP_FS_EXT_ID);
    let missing_report = missing_dependency.check();
    assert!(
        missing_report.violations.iter().any(|violation| violation
            .contains("native-fs repository profile linux: direct roots must be")),
        "{:#?}",
        missing_report.violations
    );

    let mut feature_drift = Fixture::canonical();
    feature_drift
        .repository_target_metadata
        .get_mut("x86_64-unknown-linux-gnu")
        .expect("fixture repository Linux metadata")["resolve"]["nodes"]
        .as_array_mut()
        .expect("fixture nodes")
        .iter_mut()
        .find(|node| node["id"] == CAP_FS_EXT_ID)
        .expect("fixture cap-fs-ext node")["features"] = json!([]);
    let feature_report = feature_drift.check();
    assert!(
        feature_report.violations.iter().any(|violation| violation
            == "native-fs repository profile linux: cap-fs-ext resolved features must be exactly {\"cap-std\", \"std\"}, found {}"),
        "{:#?}",
        feature_report.violations
    );

    let mut checksum_drift = Fixture::canonical();
    checksum_drift.lockfile = checksum_drift.lockfile.replace(
        "d78e5a3368ae89b7cb68186411452b4b9fac8b41be9c19bf3f47c2d2c8e36e6b",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    );
    let checksum_report = checksum_drift.check();
    assert!(
        checksum_report
            .violations
            .iter()
            .any(|violation| violation.contains("repository profile")
                && violation.contains("cap-fs-ext must resolve to 4.0.2")),
        "{:#?}",
        checksum_report.violations
    );

    let mut baseline_drift = Fixture::canonical();
    baseline_drift.repository_baseline["dependency_audit_profiles"]["linux"]["nodes"]["added"]["cap-fs-ext@4.0.2"]
        ["dependencies"] = json!([]);
    let baseline_report = baseline_drift.check();
    assert!(
        baseline_report.violations.iter().any(|violation| violation.contains(
            "repository baseline.dependency_audit_profiles.linux.nodes.added.cap-fs-ext@4.0.2.dependencies"
        )),
        "{:#?}",
        baseline_report.violations
    );
}

#[test]
fn reviewed_unsafe_boundary_semantics_are_bound_to_the_baseline() {
    let mut fixture = Fixture::canonical();
    fixture.baseline["review_basis"]["windows_directory_share_mode"]["semantic_contract"] =
        Value::String("drifted contract".to_owned());

    let report = fixture.check();

    assert!(
        report.violations.iter().any(|violation| violation
            .contains("baseline.review_basis.windows_directory_share_mode.semantic_contract")),
        "{:#?}",
        report.violations
    );
}

#[test]
fn source_and_target_root_drift_are_rejected() {
    let mut source_drift = Fixture::canonical();
    let linux = source_drift
        .target_metadata
        .get_mut("x86_64-unknown-linux-gnu")
        .expect("fixture Linux metadata");
    linux["packages"]
        .as_array_mut()
        .expect("fixture packages")
        .iter_mut()
        .find(|package| package["id"] == CAP_STD_ID)
        .expect("fixture cap-std package")["source"] =
        Value::String("git+https://example.invalid/cap-std".to_owned());
    let source_report = source_drift.check();
    assert!(
        source_report
            .violations
            .iter()
            .any(|violation| violation.contains("missing checksum for cap-std@4.0.2")),
        "{:#?}",
        source_report.violations
    );

    let mut windows_rustix = Fixture::canonical();
    add_direct_rustix(
        windows_rustix
            .target_metadata
            .get_mut("x86_64-pc-windows-msvc")
            .expect("fixture Windows metadata"),
    );
    let windows_report = windows_rustix.check();
    assert!(
        windows_report
            .violations
            .iter()
            .any(|violation| violation.contains("windows-msvc: direct roots must be")),
        "{:#?}",
        windows_report.violations
    );
}

#[test]
fn normal_build_closure_includes_build_edges_and_excludes_dev_edges() {
    const HELPER_ID: &str =
        "registry+https://github.com/rust-lang/crates.io-index#build-helper@1.0.0";
    const HELPER_LOCK: &str = r#"

[[package]]
name = "build-helper"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "1111111111111111111111111111111111111111111111111111111111111111"
"#;

    let mut build_fixture = Fixture::canonical();
    build_fixture.lockfile.push_str(HELPER_LOCK);
    let build_linux = build_fixture
        .target_metadata
        .get_mut("x86_64-unknown-linux-gnu")
        .expect("fixture Linux metadata");
    build_linux["packages"]
        .as_array_mut()
        .expect("fixture packages")
        .push(package(HELPER_ID, "build-helper", "1.0.0"));
    build_linux["resolve"]["nodes"]
        .as_array_mut()
        .expect("fixture nodes")
        .push(node(HELPER_ID, vec![], vec![]));
    build_linux["resolve"]["nodes"]
        .as_array_mut()
        .expect("fixture nodes")
        .iter_mut()
        .find(|node| node["id"] == CAP_PRIMITIVES_ID)
        .expect("fixture cap-primitives node")["deps"]
        .as_array_mut()
        .expect("fixture dependencies")
        .push(dependency_with_kind("build_helper", HELPER_ID, "build"));

    let build_report = build_fixture.check();
    assert!(
        build_report
            .violations
            .iter()
            .any(|violation| violation.contains("build-helper@1.0.0")),
        "{:#?}",
        build_report.violations
    );

    let mut dev_fixture = Fixture::canonical();
    dev_fixture.lockfile.push_str(HELPER_LOCK);
    let dev_linux = dev_fixture
        .target_metadata
        .get_mut("x86_64-unknown-linux-gnu")
        .expect("fixture Linux metadata");
    dev_linux["packages"]
        .as_array_mut()
        .expect("fixture packages")
        .push(package(HELPER_ID, "build-helper", "1.0.0"));
    dev_linux["resolve"]["nodes"]
        .as_array_mut()
        .expect("fixture nodes")
        .push(node(HELPER_ID, vec![], vec![]));
    dev_linux["resolve"]["nodes"]
        .as_array_mut()
        .expect("fixture nodes")
        .iter_mut()
        .find(|node| node["id"] == CAP_PRIMITIVES_ID)
        .expect("fixture cap-primitives node")["deps"]
        .as_array_mut()
        .expect("fixture dependencies")
        .push(dependency_with_kind("build_helper", HELPER_ID, "dev"));

    let dev_report = dev_fixture.check();
    assert!(
        dev_report.violations.is_empty(),
        "{:#?}",
        dev_report.violations
    );
}

#[test]
fn portable_byte_graph_rejects_native_filesystem_packages() {
    let mut fixture = Fixture::canonical();
    let wasm = fixture
        .target_metadata
        .get_mut("wasm32-unknown-unknown")
        .expect("fixture WASM metadata");
    wasm["packages"]
        .as_array_mut()
        .expect("fixture packages")
        .push(package(CAP_STD_ID, "cap-std", "4.0.2"));
    wasm["resolve"]["nodes"]
        .as_array_mut()
        .expect("fixture nodes")
        .push(node(CAP_STD_ID, vec![], vec![]));
    wasm["resolve"]["nodes"][0]["deps"] = json!([dependency("cap_std", CAP_STD_ID)]);

    let report = fixture.check();

    assert!(
        report.violations.iter().any(|violation| violation.contains(
            "portable profile wasm32-unknown-unknown: byte-only graph contains forbidden packages"
        )),
        "{:#?}",
        report.violations
    );
}

#[test]
fn portable_byte_graph_rejects_native_filesystem_feature_activation() {
    let mut fixture = Fixture::canonical();
    let wasm = fixture
        .target_metadata
        .get_mut("wasm32-wasip1")
        .expect("fixture WASI metadata");
    let io_id = "path+file:///fixture/baozi-io#0.1.0";
    wasm["packages"]
        .as_array_mut()
        .expect("fixture packages")
        .push(package(io_id, "baozi-io", "0.1.0"));
    wasm["resolve"]["nodes"]
        .as_array_mut()
        .expect("fixture nodes")
        .push(node(io_id, vec![], vec!["fs-candidate"]));
    wasm["resolve"]["nodes"][0]["features"] = json!(["all-formats", "native-fs-candidate"]);
    wasm["resolve"]["nodes"][0]["deps"] = json!([dependency("baozi_io", io_id)]);

    let report = fixture.check();

    assert!(
        report.violations.iter().any(|violation| violation.contains(
            "portable profile wasm32-wasip1: byte-only graph enables forbidden features"
        )),
        "{:#?}",
        report.violations
    );
}

#[test]
fn portable_byte_graph_rejects_repository_feature_activation() {
    let mut fixture = Fixture::canonical();
    let wasm = fixture
        .target_metadata
        .get_mut("wasm32-unknown-unknown")
        .expect("fixture WASM metadata");
    let io_id = "path+file:///fixture/baozi-io#0.1.0";
    wasm["packages"]
        .as_array_mut()
        .expect("fixture packages")
        .push(package(io_id, "baozi-io", "0.1.0"));
    wasm["resolve"]["nodes"]
        .as_array_mut()
        .expect("fixture nodes")
        .push(node(
            io_id,
            vec![],
            vec!["fs-candidate", "repository-fs-candidate"],
        ));
    wasm["resolve"]["nodes"][0]["deps"] = json!([dependency("baozi_io", io_id)]);

    let report = fixture.check();

    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.contains("baozi-io/repository-fs-candidate")),
        "{:#?}",
        report.violations
    );
}

#[test]
fn native_fs_cli_rejects_extra_arguments_with_usage_exit() {
    let output = Command::new(env!("CARGO_BIN_EXE_baozi-xtask"))
        .args(["native-fs-check", "unexpected"])
        .current_dir(repository_root())
        .output()
        .expect("run native filesystem dependency usage check");

    assert_eq!(output.status.code(), Some(2), "{output:#?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("usage: cargo run -p baozi-xtask"),
        "{output:#?}"
    );
}

struct Fixture {
    manifest: Value,
    local_declaration_metadata: Value,
    lockfile: String,
    baseline: Value,
    repository_baseline: Value,
    target_metadata: BTreeMap<String, Value>,
    repository_target_metadata: BTreeMap<String, Value>,
    facade_candidate_target_metadata: BTreeMap<String, Value>,
    xtask_empty_metadata: Value,
    xtask_all_metadata: Value,
}

struct SourcePolicyWorkspace {
    root: PathBuf,
}

impl SourcePolicyWorkspace {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "baozi-native-fs-source-{name}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("crates")).expect("create fixture workspace");
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nresolver = \"3\"\nmembers = [\"crates/*\"]\nexclude = [\"vendor/*\"]\n",
        )
        .expect("write fixture workspace manifest");
        let workspace = Self { root };
        workspace.write_package("crates/baozi-io", "baozi-io", "", "pub struct AssetIo;");
        workspace.write_package(
            "crates/baozi-import",
            "baozi-import",
            "",
            "pub struct ImportSession;",
        );
        workspace
    }

    fn write_package(&self, relative: &str, name: &str, package_fields: &str, source: &str) {
        let directory = self.root.join(relative);
        fs::create_dir_all(directory.join("src")).expect("create fixture package");
        fs::write(
            directory.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n{package_fields}\n"
            ),
        )
        .expect("write fixture package manifest");
        fs::write(directory.join("src/lib.rs"), source).expect("write fixture package source");
    }

    fn write_source(&self, relative: &str, source: &str) {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create fixture source parent");
        }
        fs::write(path, source).expect("write fixture source");
    }

    fn write_facade(&self, dependencies: &str, dev_dependencies: &str, all_formats: &str) {
        let directory = self.root.join("crates/baozi");
        fs::create_dir_all(directory.join("src")).expect("create facade fixture");
        fs::write(
            directory.join("Cargo.toml"),
            format!(
                "[package]\nname = \"baozi\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[features]\ndefault = []\nall-formats = [{all_formats}]\n\n[dependencies]\nbaozi-import = {{ path = \"../baozi-import\" }}\n{dependencies}\n\n[dev-dependencies]\n{dev_dependencies}\n"
            ),
        )
        .expect("write facade fixture manifest");
        fs::write(directory.join("src/lib.rs"), "pub fn import() {}").expect("write facade source");
    }

    fn source_policy_violations(&self) -> Vec<String> {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let output = Command::new(cargo)
            .args(["generate-lockfile", "--offline"])
            .current_dir(&self.root)
            .output()
            .expect("generate fixture lockfile");
        assert!(output.status.success(), "{output:#?}");
        baozi_xtask::native_fs::repository_production_source_violations(&self.root)
    }
}

impl Drop for SourcePolicyWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl Fixture {
    fn canonical() -> Self {
        let manifest = manifest_metadata();
        let local_declaration_metadata = manifest.clone();
        let lockfile = LOCKFILE.to_owned();
        let target_metadata = target_metadata();
        let baseline =
            baozi_xtask::native_fs::resolved_closure_snapshot(&target_metadata, &lockfile)
                .expect("canonical fixture snapshot");
        let repository_target_metadata = repository_target_metadata();
        let repository_baseline = baozi_xtask::native_fs::repository_resolved_closure_snapshot(
            &baseline,
            &repository_target_metadata,
            &lockfile,
        )
        .expect("canonical repository fixture snapshot");
        let facade_candidate_target_metadata = facade_candidate_target_metadata();
        let xtask_empty_metadata = xtask_empty_metadata();
        let xtask_all_metadata = xtask_all_metadata();
        Self {
            manifest,
            local_declaration_metadata,
            lockfile,
            baseline,
            repository_baseline,
            target_metadata,
            repository_target_metadata,
            facade_candidate_target_metadata,
            xtask_empty_metadata,
            xtask_all_metadata,
        }
    }

    fn check(&self) -> baozi_xtask::native_fs::NativeFsReport {
        baozi_xtask::native_fs::check_metadata_fixture_with_repository(
            &self.manifest,
            &self.lockfile,
            &self.baseline,
            &self.repository_baseline,
            &self.target_metadata,
            baozi_xtask::native_fs::RepositoryMetadataProfiles {
                local_declarations: &self.local_declaration_metadata,
                repository: &self.repository_target_metadata,
                facade: &self.facade_candidate_target_metadata,
                xtask_empty: &self.xtask_empty_metadata,
                xtask_all: &self.xtask_all_metadata,
            },
        )
    }

    fn manifest_dependencies_mut(&mut self) -> &mut Vec<Value> {
        self.manifest["packages"][0]["dependencies"]
            .as_array_mut()
            .expect("fixture manifest dependencies")
    }
}

fn manifest_metadata() -> Value {
    json!({
        "workspace_root": "/fixture",
        "workspace_members": [
            FIXTURE_IO_ID,
            FIXTURE_FACADE_ID,
            FIXTURE_XTASK_ID
        ],
        "packages": [
            {
                "id": FIXTURE_IO_ID,
                "name": "baozi-io",
                "version": "0.1.0",
                "source": null,
                "manifest_path": "/fixture/crates/baozi-io/Cargo.toml",
                "features": {
                    "default": [],
                    "fs-candidate": ["dep:cap-std", "dep:rustix"],
                    "repository-fs-candidate": ["fs-candidate", "dep:cap-fs-ext"]
                },
                "dependencies": [
                    {
                        "name": "thiserror",
                        "source": SOURCE,
                        "req": "^2",
                        "kind": null,
                        "rename": null,
                        "optional": false,
                        "uses_default_features": true,
                        "features": [],
                        "target": null
                    },
                    {
                        "name": "cap-fs-ext",
                        "source": SOURCE,
                        "req": "=4.0.2",
                        "kind": null,
                        "rename": null,
                        "optional": true,
                        "uses_default_features": false,
                        "features": ["std"],
                        "target": "cfg(any(all(target_arch = \"x86_64\", target_vendor = \"unknown\", target_os = \"linux\", target_env = \"gnu\", target_abi = \"\"), all(target_arch = \"x86_64\", target_vendor = \"pc\", target_os = \"windows\", target_env = \"msvc\", target_abi = \"\"), all(target_arch = \"aarch64\", target_vendor = \"apple\", target_os = \"macos\", target_env = \"\", target_abi = \"\"), all(target_arch = \"x86_64\", target_vendor = \"unknown\", target_os = \"freebsd\", target_env = \"\", target_abi = \"\")))"
                    },
                    {
                        "name": "cap-std",
                        "source": SOURCE,
                        "req": "=4.0.2",
                        "kind": null,
                        "rename": null,
                        "optional": true,
                        "uses_default_features": false,
                        "features": [],
                        "target": "cfg(any(all(target_arch = \"x86_64\", target_vendor = \"unknown\", target_os = \"linux\", target_env = \"gnu\", target_abi = \"\"), all(target_arch = \"x86_64\", target_vendor = \"pc\", target_os = \"windows\", target_env = \"msvc\", target_abi = \"\"), all(target_arch = \"aarch64\", target_vendor = \"apple\", target_os = \"macos\", target_env = \"\", target_abi = \"\"), all(target_arch = \"x86_64\", target_vendor = \"unknown\", target_os = \"freebsd\", target_env = \"\", target_abi = \"\")))"
                    },
                    {
                        "name": "rustix",
                        "source": SOURCE,
                        "req": "=1.1.4",
                        "kind": null,
                        "rename": null,
                        "optional": true,
                        "uses_default_features": false,
                        "features": ["std", "fs"],
                        "target": "cfg(any(all(target_arch = \"x86_64\", target_vendor = \"unknown\", target_os = \"linux\", target_env = \"gnu\", target_abi = \"\"), all(target_arch = \"aarch64\", target_vendor = \"apple\", target_os = \"macos\", target_env = \"\", target_abi = \"\"), all(target_arch = \"x86_64\", target_vendor = \"unknown\", target_os = \"freebsd\", target_env = \"\", target_abi = \"\")))"
                    }
                ]
            },
            {
                "id": FIXTURE_FACADE_ID,
                "name": "baozi",
                "version": "0.1.0",
                "source": null,
                "manifest_path": "/fixture/crates/baozi/Cargo.toml",
                "features": {
                    "default": ["default-formats", "std"],
                    "default-formats": [],
                    "all-formats": [],
                    "std": [],
                    "native-fs-candidate": ["std", "baozi-io/fs-candidate"]
                },
                "dependencies": []
            },
            {
                "id": FIXTURE_XTASK_ID,
                "name": "baozi-xtask",
                "version": "0.1.0",
                "source": null,
                "manifest_path": "/fixture/crates/baozi-xtask/Cargo.toml",
                "features": {
                    "default": [],
                    "repository-input-candidate": ["dep:baozi-io"]
                },
                "dependencies": [
                    {
                        "name": "baozi-io",
                        "source": null,
                        "req": "*",
                        "kind": null,
                        "rename": null,
                        "optional": true,
                        "uses_default_features": false,
                        "features": ["repository-fs-candidate"],
                        "target": "cfg(any(all(target_arch = \"x86_64\", target_vendor = \"unknown\", target_os = \"linux\", target_env = \"gnu\", target_abi = \"\"), all(target_arch = \"x86_64\", target_vendor = \"pc\", target_os = \"windows\", target_env = \"msvc\", target_abi = \"\"), all(target_arch = \"aarch64\", target_vendor = \"apple\", target_os = \"macos\", target_env = \"\", target_abi = \"\")))"
                    }
                ]
            }
        ]
    })
}

fn target_metadata() -> BTreeMap<String, Value> {
    let mut targets = BTreeMap::new();
    for (target, direct_rustix) in DEPENDENCY_AUDIT_TARGETS {
        targets.insert(
            (*target).to_owned(),
            native_metadata(target, *direct_rustix),
        );
    }
    for target in PORTABLE_TARGETS {
        targets.insert((*target).to_owned(), portable_metadata());
    }
    targets
}

fn repository_target_metadata() -> BTreeMap<String, Value> {
    DEPENDENCY_AUDIT_TARGETS
        .iter()
        .map(|(target, direct_rustix)| {
            let mut metadata = native_metadata(target, *direct_rustix);
            add_cap_fs_ext(&mut metadata, true);
            ((*target).to_owned(), metadata)
        })
        .collect()
}

fn facade_candidate_target_metadata() -> BTreeMap<String, Value> {
    DEPENDENCY_AUDIT_TARGETS
        .iter()
        .filter(|(target, _)| RUNTIME_CANDIDATE_TARGETS.contains(target))
        .map(|(target, direct_rustix)| {
            (
                (*target).to_owned(),
                facade_candidate_metadata(target, *direct_rustix),
            )
        })
        .collect()
}

fn facade_candidate_metadata(target: &str, direct_rustix: bool) -> Value {
    const FACADE_ID: &str = "path+file:///fixture/baozi#0.1.0";
    const IO_ID: &str = "path+file:///fixture/baozi-io#0.1.0";
    let mut metadata = native_metadata(target, direct_rustix);
    let packages = metadata["packages"]
        .as_array_mut()
        .expect("fixture packages");
    packages
        .iter_mut()
        .find(|package| package["id"] == IO_ID)
        .expect("fixture baozi-io package")["manifest_path"] =
        json!("/fixture/crates/baozi-io/Cargo.toml");
    let mut facade = package(FACADE_ID, "baozi", "0.1.0");
    facade["manifest_path"] = json!("/fixture/crates/baozi/Cargo.toml");
    packages.push(facade);
    metadata["resolve"]["nodes"]
        .as_array_mut()
        .expect("fixture nodes")
        .push(node(
            FACADE_ID,
            vec![dependency("baozi_io", IO_ID)],
            vec!["all-formats", "native-fs-candidate"],
        ));
    metadata["workspace_root"] = json!("/fixture");
    metadata["workspace_members"] = json!([FACADE_ID, IO_ID]);
    metadata
}

fn xtask_all_metadata() -> Value {
    const XTASK_ID: &str = "path+file:///fixture/baozi-xtask#0.1.0";
    const IO_ID: &str = "path+file:///fixture/baozi-io#0.1.0";
    let mut metadata = native_metadata("x86_64-unknown-linux-gnu", true);
    add_cap_fs_ext(&mut metadata, true);
    let packages = metadata["packages"]
        .as_array_mut()
        .expect("fixture packages");
    packages
        .iter_mut()
        .find(|package| package["id"] == IO_ID)
        .expect("fixture baozi-io package")["manifest_path"] =
        json!("/fixture/crates/baozi-io/Cargo.toml");
    let mut xtask_package = package(XTASK_ID, "baozi-xtask", "0.1.0");
    xtask_package["manifest_path"] = json!("/fixture/crates/baozi-xtask/Cargo.toml");
    packages.push(xtask_package);
    metadata["resolve"]["nodes"]
        .as_array_mut()
        .expect("fixture nodes")
        .push(node(
            XTASK_ID,
            vec![dependency("baozi_io", IO_ID)],
            vec!["repository-input-candidate"],
        ));
    metadata["workspace_root"] = json!("/fixture");
    metadata["workspace_members"] = json!([IO_ID, XTASK_ID]);
    metadata
}

fn xtask_empty_metadata() -> Value {
    const XTASK_ID: &str = "path+file:///fixture/baozi-xtask#0.1.0";
    let mut metadata = xtask_all_metadata();
    let root = metadata["resolve"]["nodes"]
        .as_array_mut()
        .expect("fixture nodes")
        .iter_mut()
        .find(|node| node["id"] == XTASK_ID)
        .expect("fixture xtask node");
    root["deps"] = json!([]);
    root["features"] = json!([]);
    metadata
}

fn native_metadata(target: &str, direct_rustix: bool) -> Value {
    let root_id = "path+file:///fixture/baozi-io#0.1.0";
    let mut packages = vec![
        package(root_id, "baozi-io", "0.1.0"),
        package(CAP_STD_ID, "cap-std", "4.0.2"),
        package(CAP_PRIMITIVES_ID, "cap-primitives", "4.0.2"),
    ];
    let mut root_dependencies = vec![dependency("cap_std", CAP_STD_ID)];
    let mut nodes = vec![
        node(
            CAP_STD_ID,
            vec![dependency("cap_primitives", CAP_PRIMITIVES_ID)],
            vec![],
        ),
        node(CAP_PRIMITIVES_ID, vec![], vec![]),
    ];
    if direct_rustix {
        packages.push(package(RUSTIX_ID, "rustix", "1.1.4"));
        root_dependencies.push(dependency("rustix", RUSTIX_ID));
        nodes.push(node(RUSTIX_ID, vec![], vec!["fs", "std"]));
    }
    nodes.push(node(root_id, root_dependencies, vec!["fs-candidate"]));
    json!({
        "packages": packages,
        "resolve": { "nodes": nodes },
        "target": target
    })
}

fn add_cap_fs_ext(metadata: &mut Value, repository_feature: bool) {
    metadata["packages"]
        .as_array_mut()
        .expect("fixture packages")
        .push(package(CAP_FS_EXT_ID, "cap-fs-ext", "4.0.2"));
    let nodes = metadata["resolve"]["nodes"]
        .as_array_mut()
        .expect("fixture nodes");
    let root = nodes
        .iter_mut()
        .find(|node| node["id"] == "path+file:///fixture/baozi-io#0.1.0")
        .expect("fixture root node");
    root["deps"]
        .as_array_mut()
        .expect("fixture root deps")
        .push(dependency("cap_fs_ext", CAP_FS_EXT_ID));
    if repository_feature {
        root["features"] = json!(["fs-candidate", "repository-fs-candidate"]);
    }
    nodes
        .iter_mut()
        .find(|node| node["id"] == CAP_STD_ID)
        .expect("fixture cap-std node")["features"] = json!(["default"]);
    nodes.push(node(
        CAP_FS_EXT_ID,
        vec![
            dependency("cap_primitives", CAP_PRIMITIVES_ID),
            dependency("cap_std", CAP_STD_ID),
        ],
        vec!["cap-std", "std"],
    ));
}

fn portable_metadata() -> Value {
    let root_id = "path+file:///fixture/baozi#0.1.0";
    json!({
        "packages": [package(root_id, "baozi", "0.1.0")],
        "resolve": { "nodes": [node(root_id, vec![], vec![])] }
    })
}

fn add_direct_rustix(metadata: &mut Value) {
    metadata["packages"]
        .as_array_mut()
        .expect("fixture packages")
        .push(package(RUSTIX_ID, "rustix", "1.1.4"));
    let nodes = metadata["resolve"]["nodes"]
        .as_array_mut()
        .expect("fixture nodes");
    nodes
        .iter_mut()
        .find(|node| node["id"] == "path+file:///fixture/baozi-io#0.1.0")
        .expect("fixture root node")["deps"]
        .as_array_mut()
        .expect("fixture root deps")
        .push(dependency("rustix", RUSTIX_ID));
    nodes.push(node(RUSTIX_ID, vec![], vec!["fs", "std"]));
}

fn package(id: &str, name: &str, version: &str) -> Value {
    json!({
        "id": id,
        "name": name,
        "version": version,
        "source": if id.starts_with("registry+") { Some(SOURCE) } else { None::<&str> }
    })
}

fn node(id: &str, dependencies: Vec<Value>, features: Vec<&str>) -> Value {
    json!({ "id": id, "deps": dependencies, "features": features })
}

fn dependency(name: &str, package: &str) -> Value {
    dependency_with_kind(name, package, "normal")
}

fn dependency_with_kind(name: &str, package: &str, kind: &str) -> Value {
    let kind = match kind {
        "normal" => Value::Null,
        other => Value::String(other.to_owned()),
    };
    json!({
        "name": name,
        "pkg": package,
        "dep_kinds": [{ "kind": kind, "target": null }]
    })
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("xtask crate must be under the workspace root")
        .to_path_buf()
}
