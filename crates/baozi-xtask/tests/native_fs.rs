use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const SOURCE: &str = "registry+https://github.com/rust-lang/crates.io-index";
const CAP_STD_ID: &str = "registry+https://github.com/rust-lang/crates.io-index#cap-std@4.0.3";
const CAP_FS_EXT_ID: &str =
    "registry+https://github.com/rust-lang/crates.io-index#cap-fs-ext@4.0.3";
const CAP_PRIMITIVES_ID: &str =
    "path+file:///fixture/candidates/baozi-native-io/vendor/cap-primitives#4.0.3";
const CAP_PRIMITIVES_PATH: &str = "/fixture/candidates/baozi-native-io/vendor/cap-primitives";
const CAP_PRIMITIVES_SOURCE: &str = "path+candidates/baozi-native-io/vendor/cap-primitives";
const CAP_PRIMITIVES_UPSTREAM_CHECKSUM: &str =
    "8b5f74729fd2f44701d1a8eb47e906cdb3ccd9ec0f02baad85a744b791940b18";
const CAP_PRIMITIVES_PATCHES_PATH: &str =
    "candidates/baozi-native-io/vendor/cap-primitives/PATCHES.md";
const RUSTIX_ID: &str = "registry+https://github.com/rust-lang/crates.io-index#rustix@1.1.4";
const FIXTURE_CANDIDATE_ID: &str =
    "path+file:///fixture/candidates/baozi-native-io#baozi-native-io-candidate@0.1.0";
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
version = "4.0.3"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "56ff379b70af8e08307a8f65e7040c7301cb4a572538ade16b4984f0da77847f"

[[package]]
name = "cap-primitives"
version = "4.0.3"

[[package]]
name = "cap-std"
version = "4.0.3"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "c1ec78e242cfa2cfe276807ac2ecc00315a6c97786977414bcd1c3963b6c91b8"

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
    assert_eq!(report.profiles_checked, 11);
    assert_eq!(fixture.baseline["schema"], json!(5));
    assert_eq!(fixture.repository_baseline["schema"], json!(4));
    assert_eq!(
        fixture.repository_baseline["runtime_baseline_schema"],
        json!(5)
    );
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
fn local_backend_identity_records_upstream_provenance_without_a_patched_source_checksum() {
    let fixture = Fixture::canonical();
    let identity = &fixture.baseline["package_identities"]["cap-primitives@4.0.3"];
    assert_eq!(
        identity,
        &json!({
            "source": CAP_PRIMITIVES_SOURCE,
            "upstream_source": SOURCE,
            "upstream_version": "4.0.3",
            "upstream_checksum": CAP_PRIMITIVES_UPSTREAM_CHECKSUM,
            "patches": CAP_PRIMITIVES_PATCHES_PATH,
        })
    );
    assert!(identity.get("checksum").is_none());
    for change in ["added", "changed"] {
        assert!(
            fixture.repository_baseline["package_identities"][change]
                .get("cap-primitives@4.0.3")
                .is_none()
        );
    }
    let share_mode = &fixture.baseline["review_basis"]["windows_directory_share_mode"];
    assert_eq!(share_mode["source"], json!(CAP_PRIMITIVES_SOURCE));
    assert_eq!(share_mode["patches"], json!(CAP_PRIMITIVES_PATCHES_PATH));
    assert_eq!(
        share_mode["identity_binding"],
        json!("package_identities.cap-primitives@4.0.3")
    );
    assert_eq!(
        share_mode["reviewed_path"],
        json!("candidates/baozi-native-io/vendor/cap-primitives/src/windows/fs/dir_utils.rs")
    );
    assert!(
        share_mode["semantic_contract"]
            .as_str()
            .unwrap()
            .contains("excluding FILE_SHARE_DELETE")
    );
}

#[test]
fn local_backend_provenance_and_review_identity_cannot_drift() {
    for (field, value) in [
        ("source", json!(SOURCE)),
        (
            "upstream_source",
            json!("git+https://example.invalid/cap-primitives"),
        ),
        ("upstream_version", json!("4.0.4")),
        ("upstream_checksum", json!("unreviewed-upstream")),
        ("patches", json!("vendor/PATCHES.md")),
        ("checksum", json!(CAP_PRIMITIVES_UPSTREAM_CHECKSUM)),
    ] {
        let mut fixture = Fixture::canonical();
        fixture.baseline["package_identities"]["cap-primitives@4.0.3"][field] = value;
        assert_violation(
            &fixture,
            &format!("baseline.package_identities.cap-primitives@4.0.3.{field}"),
        );
    }
    for field in ["source", "patches", "reviewed_path"] {
        let mut fixture = Fixture::canonical();
        fixture.baseline["review_basis"]["windows_directory_share_mode"][field] =
            json!("unreviewed-backend");
        assert_violation(
            &fixture,
            &format!("baseline.review_basis.windows_directory_share_mode.{field}"),
        );
    }
}

#[test]
fn repository_delta_requires_runtime_schema_five() {
    let mut fixture = Fixture::canonical();
    fixture.baseline["schema"] = json!(4);
    assert_violation(&fixture, "native-fs baseline: schema must be 5");
    let violations = baozi_xtask::native_fs::repository_resolved_closure_snapshot(
        &fixture.baseline,
        &fixture.repository,
        &fixture.lockfile,
    )
    .expect_err("old runtime schema must not produce a repository delta");
    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("runtime baseline schema must be 5")),
        "{violations:#?}"
    );

    let mut fixture = Fixture::canonical();
    fixture.repository_baseline["schema"] = json!(3);
    fixture.repository_baseline["runtime_baseline_schema"] = json!(4);
    assert_violation(&fixture, "native-fs repository baseline: schema must be 4");
    assert_violation(&fixture, "runtime_baseline_schema must be 5");
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
fn repository_native_fs_metadata_matches_reviewed_baselines() {
    let report = baozi_xtask::native_fs::check_repository(&repository_root());

    assert!(report.violations.is_empty(), "{:#?}", report.violations);
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

    for dependency in ["cap-std", "cap-primitives", "cap-fs-ext", "rustix"] {
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
        .runtime
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
        .runtime
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
        .runtime
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
        .find(|node| node["id"] == FIXTURE_CANDIDATE_ID)
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
        .repository
        .get_mut("x86_64-unknown-linux-gnu")
        .expect("fixture repository Linux metadata");
    missing_linux["resolve"]["nodes"]
        .as_array_mut()
        .expect("fixture nodes")
        .iter_mut()
        .find(|node| node["id"] == FIXTURE_CANDIDATE_ID)
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
        .repository
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
        "56ff379b70af8e08307a8f65e7040c7301cb4a572538ade16b4984f0da77847f",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    );
    let checksum_report = checksum_drift.check();
    assert!(
        checksum_report
            .violations
            .iter()
            .any(|violation| violation.contains("repository profile")
                && violation.contains("cap-fs-ext must resolve to 4.0.3")),
        "{:#?}",
        checksum_report.violations
    );

    let mut baseline_drift = Fixture::canonical();
    baseline_drift.repository_baseline["dependency_audit_profiles"]["linux"]["nodes"]["added"]["cap-fs-ext@4.0.3"]
        ["dependencies"] = json!([]);
    let baseline_report = baseline_drift.check();
    assert!(
        baseline_report.violations.iter().any(|violation| violation.contains(
            "repository baseline.dependency_audit_profiles.linux.nodes.added.cap-fs-ext@4.0.3.dependencies"
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
        .runtime
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
            .any(|violation| violation.contains("missing checksum for cap-std@4.0.3")),
        "{:#?}",
        source_report.violations
    );

    let mut windows_rustix = Fixture::canonical();
    add_direct_rustix(
        windows_rustix
            .runtime
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
        .runtime
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
        .runtime
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
fn native_fs_cli_rejects_extra_arguments_with_usage_exit() {
    let output = Command::new(env!("CARGO_BIN_EXE_baozi-xtask"))
        .args(["native-fs-check", "unexpected"])
        .current_dir(repository_root())
        .output()
        .expect("run native filesystem dependency usage check");

    assert_eq!(output.status.code(), Some(2), "{output:#?}");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("usage: cargo run --locked -p baozi-xtask"),
        "{output:#?}"
    );
}

#[test]
fn checked_in_manifests_keep_the_candidate_in_an_excluded_unpublished_workspace() {
    let root = repository_root();
    let manifest: toml::Value =
        toml::from_str(&std::fs::read_to_string(root.join("Cargo.toml")).expect("root manifest"))
            .expect("root TOML");
    assert!(
        manifest["workspace"]["exclude"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path.as_str() == Some("candidates/baozi-native-io"))
    );
    let candidate: toml::Value = toml::from_str(
        &std::fs::read_to_string(root.join("candidates/baozi-native-io/Cargo.toml"))
            .expect("candidate manifest"),
    )
    .expect("candidate TOML");
    assert_eq!(
        candidate["package"]["name"].as_str(),
        Some("baozi-native-io-candidate")
    );
    assert_eq!(candidate["package"]["publish"].as_bool(), Some(false));
    assert_eq!(candidate["workspace"]["resolver"].as_str(), Some("3"));
    assert!(
        !candidate["dependencies"]
            .as_table()
            .unwrap()
            .contains_key("baozi")
    );
}

#[test]
fn removed_switches_are_rejected_even_as_empty_or_forwarding_aliases() {
    for (owner, feature) in [
        ("baozi-io", "fs"),
        ("baozi-io", "fs-candidate"),
        ("baozi-io", "repository-fs-candidate"),
        ("baozi", "native-fs"),
        ("baozi", "native-fs-candidate"),
        ("baozi-xtask", "repository-input-candidate"),
    ] {
        for values in [json!([]), json!(["default"])] {
            let mut fixture = Fixture::canonical();
            local_package_mut(&mut fixture.root, owner)["features"][feature] = values;
            assert_violation(
                &fixture,
                &format!("{owner}/{feature} must be removed without an alias"),
            );
        }
    }
    let mut candidate = Fixture::canonical();
    candidate.candidate_mut()["features"]["fs-candidate"] = json!([]);
    assert_violation(&candidate, "features must be exactly");
}

#[test]
fn candidate_features_and_direct_backend_edges_are_exact() {
    let mut feature = Fixture::canonical();
    feature.candidate_mut()["features"]["repository-capture"] = json!([]);
    assert_violation(&feature, "features must be exactly");

    for (name, field, value) in [
        ("cap-std", "optional", json!(true)),
        ("rustix", "uses_default_features", json!(true)),
        ("cap-fs-ext", "optional", json!(false)),
        ("cap-std", "req", json!("^4")),
        ("cap-std", "path", json!("/fixture/vendor/cap-std")),
        ("cap-primitives", "req", json!("^4.0.3")),
        ("cap-primitives", "source", json!(SOURCE)),
        ("cap-primitives", "optional", json!(true)),
        ("cap-primitives", "uses_default_features", json!(true)),
        ("cap-primitives", "features", json!(["fs_utf8"])),
        ("cap-primitives", "kind", json!("build")),
        ("cap-primitives", "rename", json!("backend")),
        ("cap-primitives", "target", Value::Null),
        ("rustix", "features", json!(["fs", "std", "net"])),
        (
            "cap-fs-ext",
            "source",
            json!("git+https://example.invalid/cap-fs-ext"),
        ),
    ] {
        let mut fixture = Fixture::canonical();
        fixture
            .manifest_dependencies_mut()
            .iter_mut()
            .find(|dep| dep["name"] == name)
            .expect("backend declaration")[field] = value;
        assert_violation(&fixture, &format!("direct {name} edge drifted"));
    }
    let mut extra = Fixture::canonical();
    extra
        .manifest_dependencies_mut()
        .push(json!({"name": "cap-primitives", "kind": null}));
    assert_violation(&extra, "production dependencies must be exactly");
}

#[test]
fn local_backend_declaration_requires_the_reviewed_absolute_directory() {
    for path in [
        Value::Null,
        json!("vendor/cap-primitives"),
        json!("/fixture/vendor/cap-primitives"),
        json!("/fixture/candidates/baozi-native-io/vendor/cap-primitives-copy"),
        json!(format!("{CAP_PRIMITIVES_PATH}/Cargo.toml")),
        json!(format!("{CAP_PRIMITIVES_PATH}/../replacement")),
    ] {
        let mut fixture = Fixture::canonical();
        fixture
            .manifest_dependencies_mut()
            .iter_mut()
            .find(|dependency| dependency["name"] == "cap-primitives")
            .expect("local backend declaration")["path"] = path;
        assert_violation(&fixture, "direct cap-primitives edge drifted");
    }
    let mut fixture = Fixture::canonical();
    fixture
        .manifest_dependencies_mut()
        .retain(|dependency| dependency["name"] != "cap-primitives");
    assert_violation(
        &fixture,
        "expected exactly one direct cap-primitives edge, found 0",
    );
}

#[test]
fn local_backend_declaration_path_is_checked_in_every_dependency_profile() {
    for repository in [false, true] {
        for (target, _) in DEPENDENCY_AUDIT_TARGETS {
            let mut fixture = Fixture::canonical();
            let profiles = if repository {
                &mut fixture.repository
            } else {
                &mut fixture.runtime
            };
            let candidate = local_package_mut(
                profiles.get_mut(*target).unwrap(),
                "baozi-native-io-candidate",
            );
            candidate["dependencies"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|dependency| dependency["name"] == "cap-primitives")
                .expect("local backend declaration")["path"] = json!("/fixture/unreviewed-backend");
            assert_violation(&fixture, "direct cap-primitives edge drifted");
        }
    }
}

#[test]
fn resolved_local_backend_path_source_and_version_are_exact_in_every_profile() {
    for repository in [false, true] {
        for (target, _) in DEPENDENCY_AUDIT_TARGETS {
            for (field, value) in [
                ("version", json!("4.0.4")),
                ("source", json!(SOURCE)),
                (
                    "source",
                    json!("git+https://example.invalid/cap-primitives"),
                ),
                ("manifest_path", Value::Null),
                (
                    "manifest_path",
                    json!("/fixture/vendor/cap-primitives/Cargo.toml"),
                ),
                (
                    "manifest_path",
                    json!(format!("{CAP_PRIMITIVES_PATH}-copy/Cargo.toml")),
                ),
                (
                    "manifest_path",
                    json!(format!("{CAP_PRIMITIVES_PATH}/../replacement/Cargo.toml")),
                ),
            ] {
                let mut fixture = Fixture::canonical();
                let profiles = if repository {
                    &mut fixture.repository
                } else {
                    &mut fixture.runtime
                };
                local_package_mut(profiles.get_mut(*target).unwrap(), "cap-primitives")[field] =
                    value;
                assert_violation(
                    &fixture,
                    "cap-primitives must resolve to local cap-primitives@4.0.3",
                );
            }
        }
    }
}

#[test]
fn local_backend_must_also_be_a_direct_normal_resolved_dependency() {
    for repository in [false, true] {
        for change in ["missing", "renamed", "build", "dev"] {
            let mut fixture = Fixture::canonical();
            let profiles = if repository {
                &mut fixture.repository
            } else {
                &mut fixture.runtime
            };
            let root = resolved_node_mut(
                profiles.get_mut("x86_64-unknown-linux-gnu").unwrap(),
                FIXTURE_CANDIDATE_ID,
            );
            let dependencies = root["deps"].as_array_mut().unwrap();
            if change == "missing" {
                dependencies.retain(|dependency| dependency["pkg"] != CAP_PRIMITIVES_ID);
            } else {
                let dependency = dependencies
                    .iter_mut()
                    .find(|dependency| dependency["pkg"] == CAP_PRIMITIVES_ID)
                    .expect("direct local backend edge");
                if change == "renamed" {
                    dependency["name"] = json!("backend");
                } else {
                    dependency["dep_kinds"][0]["kind"] = json!(change);
                }
            }
            assert_violation(
                &fixture,
                "candidate must depend directly on local cap-primitives",
            );
        }
    }
}

#[test]
fn registry_backends_cannot_be_replaced_by_additional_local_packages() {
    for (name, version) in [
        ("cap-std", "4.0.3"),
        ("cap-fs-ext", "4.0.3"),
        ("rustix", "1.1.4"),
    ] {
        let mut fixture = Fixture::canonical();
        let metadata = fixture
            .repository
            .get_mut("x86_64-unknown-linux-gnu")
            .unwrap();
        let package = local_package_mut(metadata, name);
        package["source"] = Value::Null;
        package["manifest_path"] = json!(format!(
            "/fixture/candidates/baozi-native-io/vendor/{name}/Cargo.toml"
        ));
        assert_violation(
            &fixture,
            &format!("full-closure package {name}@{version} must come from {SOURCE}"),
        );
    }
}

#[test]
fn local_helpers_inside_the_reviewed_backend_directory_are_not_admitted() {
    const HELPER_ID: &str =
        "path+file:///fixture/candidates/baozi-native-io/vendor/cap-primitives/helper#1.0.0";
    for kind in ["normal", "build"] {
        let mut fixture = Fixture::canonical();
        let metadata = fixture.runtime.get_mut("x86_64-unknown-linux-gnu").unwrap();
        let mut helper = package(HELPER_ID, "local-helper", "1.0.0");
        helper["manifest_path"] = json!(format!("{CAP_PRIMITIVES_PATH}/helper/Cargo.toml"));
        metadata["packages"].as_array_mut().unwrap().push(helper);
        metadata["resolve"]["nodes"]
            .as_array_mut()
            .unwrap()
            .push(node(HELPER_ID, vec![], vec![]));
        resolved_node_mut(metadata, CAP_PRIMITIVES_ID)["deps"]
            .as_array_mut()
            .unwrap()
            .push(dependency_with_kind("local_helper", HELPER_ID, kind));
        assert_violation(
            &fixture,
            &format!("full-closure package local-helper@1.0.0 must come from {SOURCE}"),
        );
    }
}

#[test]
fn cap_std_and_cap_fs_ext_cannot_resolve_a_second_backend_provider() {
    const REGISTRY_CAP_PRIMITIVES_ID: &str =
        "registry+https://github.com/rust-lang/crates.io-index#cap-primitives@4.0.3";
    for owner in [CAP_STD_ID, CAP_FS_EXT_ID] {
        let mut fixture = Fixture::canonical();
        let metadata = fixture
            .repository
            .get_mut("x86_64-unknown-linux-gnu")
            .unwrap();
        metadata["packages"].as_array_mut().unwrap().push(package(
            REGISTRY_CAP_PRIMITIVES_ID,
            "cap-primitives",
            "4.0.3",
        ));
        metadata["resolve"]["nodes"]
            .as_array_mut()
            .unwrap()
            .push(node(REGISTRY_CAP_PRIMITIVES_ID, vec![], vec![]));
        resolved_node_mut(metadata, owner)["deps"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|dependency| dependency["name"] == "cap_primitives")
            .expect("transitive backend edge")["pkg"] = json!(REGISTRY_CAP_PRIMITIVES_ID);
        assert_violation(
            &fixture,
            "duplicate full-closure package key cap-primitives@4.0.3",
        );
    }
}

#[test]
fn root_declarations_reject_native_edges_of_every_kind_even_when_renamed_or_inactive() {
    for owner in ["baozi", "baozi-io", "baozi-xtask", "dev-helper"] {
        for name in [
            "cap-std",
            "cap-primitives",
            "cap-fs-ext",
            "rustix",
            "baozi-native-io-candidate",
        ] {
            for kind in [Value::Null, json!("build"), json!("dev")] {
                let mut fixture = Fixture::canonical();
                fixture.root["packages"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({
                        "name": owner, "source": null, "features": {},
                        "dependencies": [{"name": name, "rename": "hidden", "optional": true,
                            "kind": kind, "target": "cfg(target_os = \"freebsd\")"}]
                    }));
                assert_violation(&fixture, &format!("{owner} must not depend on {name}"));
            }
        }
    }
}

#[test]
fn root_declarations_reject_requests_and_weak_forwards_for_removed_io_features() {
    for feature in ["fs", "fs-candidate", "repository-fs-candidate"] {
        let mut fixture = Fixture::canonical();
        fixture.root["packages"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "name": "excluded-helper", "source": null,
                "features": {"hidden": [format!("asset_io?/{feature}")]},
                "dependencies": [{"name": "baozi-io", "rename": "asset_io", "kind": "dev",
                    "optional": true, "features": [feature]}]
            }));
        assert_violation(
            &fixture,
            &format!("must not request removed baozi-io/{feature}"),
        );
        assert_violation(
            &fixture,
            &format!("must not forward removed baozi-io/{feature}"),
        );
    }
}

#[test]
fn xtask_remains_featureless_and_cannot_declare_asset_io_even_for_tests() {
    let mut feature = Fixture::canonical();
    local_package_mut(&mut feature.root, "baozi-xtask")["features"]["helper"] = json!([]);
    assert_violation(&feature, "baozi-xtask must remain featureless");
    for kind in [Value::Null, json!("build"), json!("dev")] {
        let mut fixture = Fixture::canonical();
        local_package_mut(&mut fixture.root, "baozi-xtask")["dependencies"] = json!([
            {"name": "baozi-io", "rename": "asset_io", "kind": kind, "features": []}
        ]);
        assert_violation(&fixture, "baozi-xtask must not depend on baozi-io");
    }
}

#[test]
fn xtask_reachability_includes_normal_and_build_edges_and_excludes_dev_edges() {
    const HELPER: &str = "path+file:///fixture/helper#0.1.0";
    for kind in ["normal", "build", "dev"] {
        let mut fixture = Fixture::canonical();
        fixture.root["packages"]
            .as_array_mut()
            .unwrap()
            .push(package(HELPER, "helper", "0.1.0"));
        resolved_node_mut(&mut fixture.root, FIXTURE_XTASK_ID)["deps"] =
            json!([dependency_with_kind("helper", HELPER, kind)]);
        fixture.root["resolve"]["nodes"]
            .as_array_mut()
            .unwrap()
            .push(node(
                HELPER,
                vec![dependency("asset_io", FIXTURE_IO_ID)],
                vec![],
            ));
        let report = fixture.check();
        if kind == "dev" {
            assert!(report.violations.is_empty(), "{:#?}", report.violations);
        } else {
            assert!(
                report
                    .violations
                    .iter()
                    .any(|v| v.contains("xtask must not reach baozi-io")),
                "{:#?}",
                report.violations
            );
        }
    }
}

#[test]
fn candidate_identity_requires_its_own_unpublished_workspace_and_the_real_portable_io() {
    for field in [
        "workspace_root",
        "workspace_members",
        "resolve_root",
        "manifest_path",
        "publish",
        "io_path",
        "io_source",
    ] {
        let mut fixture = Fixture::canonical();
        let metadata = fixture.runtime.get_mut("x86_64-unknown-linux-gnu").unwrap();
        let expected = match field {
            "workspace_root" => {
                metadata["workspace_root"] = json!("/fixture");
                "workspace_root must be the standalone"
            }
            "workspace_members" => {
                metadata["workspace_members"] = json!([FIXTURE_CANDIDATE_ID, FIXTURE_IO_ID]);
                "candidate must be the only member"
            }
            "resolve_root" => {
                metadata["resolve"]["root"] = json!(FIXTURE_IO_ID);
                "candidate must be the resolve root"
            }
            "manifest_path" => {
                local_package_mut(metadata, "baozi-native-io-candidate")["manifest_path"] =
                    json!("/fixture/Cargo.toml");
                "manifest_path must be"
            }
            "publish" => {
                local_package_mut(metadata, "baozi-native-io-candidate")["publish"] = Value::Null;
                "must set publish = false"
            }
            "io_path" => {
                local_package_mut(metadata, "baozi-io")["manifest_path"] =
                    json!("/fixture/vendor/baozi-io/Cargo.toml");
                "baozi-io must be the portable path package"
            }
            "io_source" => {
                local_package_mut(metadata, "baozi-io")["source"] = json!(SOURCE);
                "baozi-io must be the portable path package"
            }
            _ => unreachable!(),
        };
        assert_violation(&fixture, expected);
    }
    let mut fixture = Fixture::canonical();
    fixture
        .manifest_dependencies_mut()
        .iter_mut()
        .find(|dep| dep["name"] == "baozi-io")
        .unwrap()["path"] = json!("/fixture/vendor/baozi-io");
    assert_violation(&fixture, "must depend unconditionally on portable baozi-io");
}

#[test]
fn candidate_runtime_and_repository_features_cannot_unify() {
    let mut runtime = Fixture::canonical();
    resolved_node_mut(
        runtime.runtime.get_mut("x86_64-unknown-linux-gnu").unwrap(),
        FIXTURE_CANDIDATE_ID,
    )["features"] = json!(["repository-capture"]);
    assert_violation(
        &runtime,
        "baozi-native-io-candidate resolved features must be exactly []",
    );
    let mut repository = Fixture::canonical();
    resolved_node_mut(
        repository
            .repository
            .get_mut("x86_64-unknown-linux-gnu")
            .unwrap(),
        FIXTURE_CANDIDATE_ID,
    )["features"] = json!([]);
    assert_violation(
        &repository,
        "baozi-native-io-candidate resolved features must be exactly [\"repository-capture\"]",
    );
}

#[test]
fn portable_audit_starts_at_all_workspace_members_and_rejects_native_normal_and_build_edges() {
    for kind in ["normal", "build", "dev"] {
        let mut fixture = Fixture::canonical();
        let wasm = fixture.portable.get_mut("wasm32-unknown-unknown").unwrap();
        wasm["packages"]
            .as_array_mut()
            .unwrap()
            .push(package(CAP_STD_ID, "cap-std", "4.0.3"));
        wasm["resolve"]["nodes"]
            .as_array_mut()
            .unwrap()
            .push(node(CAP_STD_ID, vec![], vec![]));
        // IO is a workspace member even when the facade does not reach it.
        resolved_node_mut(wasm, FIXTURE_FACADE_ID)["deps"] = json!([]);
        resolved_node_mut(wasm, FIXTURE_IO_ID)["deps"] =
            json!([dependency_with_kind("cap_std", CAP_STD_ID, kind)]);
        let report = fixture.check();
        if kind == "dev" {
            assert!(report.violations.is_empty(), "{:#?}", report.violations);
        } else {
            assert!(
                report
                    .violations
                    .iter()
                    .any(|v| v
                        .contains("portable workspace contains forbidden packages {\"cap-std\"}")),
                "{:#?}",
                report.violations
            );
        }
    }
    let mut fixture = Fixture::canonical();
    let wasi = fixture.portable.get_mut("wasm32-wasip1").unwrap();
    resolved_node_mut(wasi, FIXTURE_IO_ID)["features"] = json!(["repository-fs-candidate"]);
    assert_violation(&fixture, "baozi-io/repository-fs-candidate");
}

#[test]
fn missing_targets_and_lock_checksums_fail_closed() {
    let mut fixture = Fixture::canonical();
    fixture.runtime.remove("x86_64-unknown-freebsd");
    fixture.repository.remove("aarch64-apple-darwin");
    fixture.portable.remove("wasm32-wasip1");
    for expected in [
        "profile freebsd: missing cargo metadata",
        "repository profile macos: missing cargo metadata",
        "portable profile wasm32-wasip1: missing cargo metadata",
    ] {
        assert_violation(&fixture, expected);
    }
    let mut fixture = Fixture::canonical();
    fixture.lockfile =
        "version = 4\n[[package]]\nname = 'unrelated'\nversion = '1.0.0'\n".to_owned();
    assert_violation(&fixture, "missing checksum for cap-");
}

struct Fixture {
    root: Value,
    lockfile: String,
    baseline: Value,
    repository_baseline: Value,
    runtime: BTreeMap<String, Value>,
    repository: BTreeMap<String, Value>,
    portable: BTreeMap<String, Value>,
}

impl Fixture {
    fn canonical() -> Self {
        let root = root_metadata();
        let lockfile = LOCKFILE.to_owned();
        let runtime = DEPENDENCY_AUDIT_TARGETS
            .iter()
            .map(|(target, rustix)| ((*target).to_owned(), native_metadata(*rustix)))
            .collect();
        let portable = PORTABLE_TARGETS
            .iter()
            .map(|target| ((*target).to_owned(), root.clone()))
            .collect();
        let baseline =
            baozi_xtask::native_fs::resolved_closure_snapshot(&runtime, &portable, &lockfile)
                .expect("canonical runtime snapshot");
        let repository = DEPENDENCY_AUDIT_TARGETS
            .iter()
            .map(|(target, rustix)| {
                let mut metadata = native_metadata(*rustix);
                add_cap_fs_ext(&mut metadata, true);
                ((*target).to_owned(), metadata)
            })
            .collect();
        let repository_baseline = baozi_xtask::native_fs::repository_resolved_closure_snapshot(
            &baseline,
            &repository,
            &lockfile,
        )
        .expect("canonical repository snapshot");
        Self {
            root,
            lockfile,
            baseline,
            repository_baseline,
            runtime,
            repository,
            portable,
        }
    }

    fn check(&self) -> baozi_xtask::native_fs::NativeFsReport {
        baozi_xtask::native_fs::check_metadata_fixture(
            &self.lockfile,
            &self.baseline,
            &self.repository_baseline,
            baozi_xtask::native_fs::NativeFsMetadataProfiles {
                root: &self.root,
                runtime: &self.runtime,
                repository: &self.repository,
                portable: &self.portable,
            },
        )
    }

    fn candidate_mut(&mut self) -> &mut Value {
        local_package_mut(
            self.runtime.get_mut("x86_64-unknown-linux-gnu").unwrap(),
            "baozi-native-io-candidate",
        )
    }

    fn manifest_dependencies_mut(&mut self) -> &mut Vec<Value> {
        self.candidate_mut()["dependencies"].as_array_mut().unwrap()
    }
}

fn assert_violation(fixture: &Fixture, expected: &str) {
    let report = fixture.check();
    assert!(
        report.violations.iter().any(|v| v.contains(expected)),
        "missing {expected:?}: {:#?}",
        report.violations
    );
}

fn local_package_mut<'a>(metadata: &'a mut Value, name: &str) -> &'a mut Value {
    metadata["packages"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|package| package["name"] == name)
        .expect("local package")
}

fn resolved_node_mut<'a>(metadata: &'a mut Value, id: &str) -> &'a mut Value {
    metadata["resolve"]["nodes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|node| node["id"] == id)
        .expect("resolved node")
}

fn root_metadata() -> Value {
    let mut io = package(FIXTURE_IO_ID, "baozi-io", "0.1.0");
    io["manifest_path"] = json!("/fixture/crates/baozi-io/Cargo.toml");
    io["features"] = json!({"default": []});
    let mut facade = package(FIXTURE_FACADE_ID, "baozi", "0.1.0");
    facade["manifest_path"] = json!("/fixture/crates/baozi/Cargo.toml");
    facade["features"] = json!({"default": ["all-formats"], "all-formats": []});
    facade["dependencies"] = json!([{"name": "baozi-io", "features": [], "rename": null}]);
    let mut xtask = package(FIXTURE_XTASK_ID, "baozi-xtask", "0.1.0");
    xtask["manifest_path"] = json!("/fixture/crates/baozi-xtask/Cargo.toml");
    json!({
        "workspace_root": "/fixture",
        "workspace_members": [FIXTURE_IO_ID, FIXTURE_FACADE_ID, FIXTURE_XTASK_ID],
        "packages": [io, facade, xtask],
        "resolve": {"root": null, "nodes": [
            node(FIXTURE_FACADE_ID, vec![dependency("baozi_io", FIXTURE_IO_ID)], vec!["all-formats"]),
            node(FIXTURE_IO_ID, vec![], vec![]),
            node(FIXTURE_XTASK_ID, vec![], vec![])
        ]}
    })
}

fn native_metadata(direct_rustix: bool) -> Value {
    let cap_target = concat!(
        "cfg(any(",
        "all(target_arch = \"x86_64\", target_vendor = \"unknown\", target_os = \"linux\", target_env = \"gnu\", target_abi = \"\"), ",
        "all(target_arch = \"x86_64\", target_vendor = \"pc\", target_os = \"windows\", target_env = \"msvc\", target_abi = \"\"), ",
        "all(target_arch = \"aarch64\", target_vendor = \"apple\", target_os = \"macos\", target_env = \"\", target_abi = \"\"), ",
        "all(target_arch = \"x86_64\", target_vendor = \"unknown\", target_os = \"freebsd\", target_env = \"\", target_abi = \"\")))"
    );
    let rustix_target = concat!(
        "cfg(any(",
        "all(target_arch = \"x86_64\", target_vendor = \"unknown\", target_os = \"linux\", target_env = \"gnu\", target_abi = \"\"), ",
        "all(target_arch = \"aarch64\", target_vendor = \"apple\", target_os = \"macos\", target_env = \"\", target_abi = \"\"), ",
        "all(target_arch = \"x86_64\", target_vendor = \"unknown\", target_os = \"freebsd\", target_env = \"\", target_abi = \"\")))"
    );
    let mut candidate = package(FIXTURE_CANDIDATE_ID, "baozi-native-io-candidate", "0.1.0");
    candidate["manifest_path"] = json!("/fixture/candidates/baozi-native-io/Cargo.toml");
    candidate["publish"] = json!([]);
    candidate["features"] = json!({"default": [], "repository-capture": ["dep:cap-fs-ext"]});
    let mut cap_primitives =
        backend_declaration("cap-primitives", "=4.0.3", cap_target, false, vec![]);
    cap_primitives["source"] = Value::Null;
    cap_primitives["path"] = json!(CAP_PRIMITIVES_PATH);
    candidate["dependencies"] = json!([
        {"name": "baozi-io", "path": "/fixture/crates/baozi-io", "source": null,
            "kind": null, "rename": null, "target": null, "optional": false,
            "uses_default_features": false, "features": []},
        {"name": "thiserror", "kind": null},
        backend_declaration("cap-std", "=4.0.3", cap_target, false, vec![]),
        cap_primitives,
        backend_declaration("cap-fs-ext", "=4.0.3", cap_target, true, vec!["std"]),
        backend_declaration("rustix", "=1.1.4", rustix_target, false, vec!["fs", "std"])
    ]);
    let io = root_metadata()["packages"][0].clone();
    let mut local_backend = package(CAP_PRIMITIVES_ID, "cap-primitives", "4.0.3");
    local_backend["manifest_path"] = json!(format!("{CAP_PRIMITIVES_PATH}/Cargo.toml"));
    let mut packages = vec![
        candidate,
        io,
        package(CAP_STD_ID, "cap-std", "4.0.3"),
        local_backend,
    ];
    let mut root_deps = vec![
        dependency("baozi_io", FIXTURE_IO_ID),
        dependency("cap_std", CAP_STD_ID),
        dependency("cap_primitives", CAP_PRIMITIVES_ID),
    ];
    let mut nodes = vec![
        node(FIXTURE_IO_ID, vec![], vec![]),
        node(
            CAP_STD_ID,
            vec![dependency("cap_primitives", CAP_PRIMITIVES_ID)],
            vec![],
        ),
        node(CAP_PRIMITIVES_ID, vec![], vec![]),
    ];
    if direct_rustix {
        packages.push(package(RUSTIX_ID, "rustix", "1.1.4"));
        root_deps.push(dependency("rustix", RUSTIX_ID));
        nodes.push(node(RUSTIX_ID, vec![], vec!["fs", "std"]));
    }
    nodes.push(node(FIXTURE_CANDIDATE_ID, root_deps, vec![]));
    json!({"workspace_root": "/fixture/candidates/baozi-native-io",
        "workspace_members": [FIXTURE_CANDIDATE_ID], "packages": packages,
        "resolve": {"root": FIXTURE_CANDIDATE_ID, "nodes": nodes}})
}

fn backend_declaration(
    name: &str,
    req: &str,
    target: &str,
    optional: bool,
    features: Vec<&str>,
) -> Value {
    json!({"name": name, "source": SOURCE, "req": req, "target": target,
        "optional": optional, "features": features, "uses_default_features": false,
        "kind": null, "rename": null})
}

fn add_cap_fs_ext(metadata: &mut Value, repository_feature: bool) {
    metadata["packages"].as_array_mut().unwrap().push(package(
        CAP_FS_EXT_ID,
        "cap-fs-ext",
        "4.0.3",
    ));
    let root = resolved_node_mut(metadata, FIXTURE_CANDIDATE_ID);
    root["deps"]
        .as_array_mut()
        .unwrap()
        .push(dependency("cap_fs_ext", CAP_FS_EXT_ID));
    if repository_feature {
        root["features"] = json!(["repository-capture"]);
    }
    resolved_node_mut(metadata, CAP_STD_ID)["features"] = json!(["default"]);
    metadata["resolve"]["nodes"]
        .as_array_mut()
        .unwrap()
        .push(node(
            CAP_FS_EXT_ID,
            vec![
                dependency("cap_primitives", CAP_PRIMITIVES_ID),
                dependency("cap_std", CAP_STD_ID),
            ],
            vec!["cap-std", "std"],
        ));
}

fn add_direct_rustix(metadata: &mut Value) {
    metadata["packages"]
        .as_array_mut()
        .unwrap()
        .push(package(RUSTIX_ID, "rustix", "1.1.4"));
    resolved_node_mut(metadata, FIXTURE_CANDIDATE_ID)["deps"]
        .as_array_mut()
        .unwrap()
        .push(dependency("rustix", RUSTIX_ID));
    metadata["resolve"]["nodes"]
        .as_array_mut()
        .unwrap()
        .push(node(RUSTIX_ID, vec![], vec!["fs", "std"]));
}

fn package(id: &str, name: &str, version: &str) -> Value {
    json!({"id": id, "name": name, "version": version,
        "source": if id.starts_with("registry+") { Some(SOURCE) } else { None::<&str> },
        "features": {}, "dependencies": []})
}

fn node(id: &str, deps: Vec<Value>, features: Vec<&str>) -> Value {
    json!({"id": id, "deps": deps, "features": features})
}

fn dependency(name: &str, package: &str) -> Value {
    dependency_with_kind(name, package, "normal")
}

fn dependency_with_kind(name: &str, package: &str, kind: &str) -> Value {
    json!({"name": name, "pkg": package, "dep_kinds": [{
        "kind": if kind == "normal" { Value::Null } else { json!(kind) }, "target": null
    }]})
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("xtask crate under workspace")
        .to_path_buf()
}
