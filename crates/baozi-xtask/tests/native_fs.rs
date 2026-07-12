use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const SOURCE: &str = "registry+https://github.com/rust-lang/crates.io-index";
const CAP_STD_ID: &str = "registry+https://github.com/rust-lang/crates.io-index#cap-std@4.0.2";
const CAP_PRIMITIVES_ID: &str =
    "registry+https://github.com/rust-lang/crates.io-index#cap-primitives@4.0.2";
const RUSTIX_ID: &str = "registry+https://github.com/rust-lang/crates.io-index#rustix@1.1.4";
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
    assert_eq!(report.profiles_checked, 6);
    assert_eq!(fixture.baseline["schema"], json!(2));
    assert_eq!(
        fixture.baseline["runtime_candidate_targets"],
        json!(RUNTIME_CANDIDATE_TARGETS)
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
fn candidate_filesystem_tests_cannot_skip_missing_fixtures() {
    let canonical = include_str!("../../baozi-io/src/fs.rs");
    assert!(baozi_xtask::native_fs::candidate_test_source_violations(canonical).is_empty());

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
fn source_injection_macros_are_rejected_without_expansion() {
    for (case, hostile) in [
        (
            "indirect include macro",
            r#"passthrough!(include!("generated.rs"));"#,
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

    for dependency in ["cap-std", "rustix"] {
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
    for forbidden in ["cap-primitives", "cap-fs-ext"] {
        let mut fixture = Fixture::canonical();
        fixture
            .manifest_dependencies_mut()
            .push(json!({ "name": forbidden }));

        let report = fixture.check();

        assert!(
            report.violations.iter().any(|violation| violation
                == &format!(
                    "native-fs manifest: baozi-io must not depend directly on {forbidden}"
                )),
            "{:#?}",
            report.violations
        );
    }
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
    lockfile: String,
    baseline: Value,
    target_metadata: BTreeMap<String, Value>,
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
        let lockfile = LOCKFILE.to_owned();
        let target_metadata = target_metadata();
        let baseline =
            baozi_xtask::native_fs::resolved_closure_snapshot(&target_metadata, &lockfile)
                .expect("canonical fixture snapshot");
        Self {
            manifest,
            lockfile,
            baseline,
            target_metadata,
        }
    }

    fn check(&self) -> baozi_xtask::native_fs::NativeFsReport {
        baozi_xtask::native_fs::check_metadata_fixture(
            &self.manifest,
            &self.lockfile,
            &self.baseline,
            &self.target_metadata,
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
        "packages": [
            {
                "name": "baozi-io",
                "features": {
                    "default": [],
                    "fs-candidate": ["dep:cap-std", "dep:rustix"]
                },
                "dependencies": [
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
                "name": "baozi",
                "features": {
                    "default": ["default-formats", "std"],
                    "default-formats": [],
                    "all-formats": [],
                    "std": [],
                    "native-fs-candidate": ["std", "baozi-io/fs-candidate"]
                },
                "dependencies": []
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
