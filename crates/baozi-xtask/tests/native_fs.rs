use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const SOURCE: &str = "registry+https://github.com/rust-lang/crates.io-index";
const CAP_STD_ID: &str = "registry+https://github.com/rust-lang/crates.io-index#cap-std@4.0.3";
const CAP_FS_EXT_ID: &str =
    "registry+https://github.com/rust-lang/crates.io-index#cap-fs-ext@4.0.3";
const CAP_PRIMITIVES_ID: &str =
    "registry+https://github.com/rust-lang/crates.io-index#cap-primitives@4.0.3";
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
version = "4.0.3"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "56ff379b70af8e08307a8f65e7040c7301cb4a572538ade16b4984f0da77847f"

[[package]]
name = "cap-primitives"
version = "4.0.3"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "8b5f74729fd2f44701d1a8eb47e906cdb3ccd9ec0f02baad85a744b791940b18"

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
    fixture.manifest["packages"][1]["features"]["native-fs-candidate"] = json!([]);

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
    fixture.manifest["packages"][1]["features"]["repository-helper"] =
        json!(["baozi-io/repository-fs-candidate"]);
    fixture.manifest["packages"][1]["features"]["native-fs-candidate"] =
        json!(["baozi-io/fs-candidate", "repository-helper"]);

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
        json!(["baozi-io/fs-candidate"]);
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
            .any(|violation| violation.contains("missing checksum for cap-std@4.0.3")),
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
        .push(package(CAP_STD_ID, "cap-std", "4.0.3"));
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
        String::from_utf8_lossy(&output.stderr)
            .contains("usage: cargo run --locked -p baozi-xtask"),
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
                        "name": "sha2",
                        "source": SOURCE,
                        "req": "^0.10",
                        "kind": null,
                        "rename": null,
                        "optional": false,
                        "uses_default_features": true,
                        "features": [],
                        "target": null
                    },
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
                        "req": "=4.0.3",
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
                        "req": "=4.0.3",
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
                    "default": ["default-formats"],
                    "default-formats": [],
                    "all-formats": [],
                    "native-fs-candidate": ["baozi-io/fs-candidate"]
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
        package(CAP_STD_ID, "cap-std", "4.0.3"),
        package(CAP_PRIMITIVES_ID, "cap-primitives", "4.0.3"),
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
        .push(package(CAP_FS_EXT_ID, "cap-fs-ext", "4.0.3"));
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
