use baozi_xtask::runtime_admission::check_metadata;
use serde_json::{Value, json};

#[test]
fn runtime_admission_accepts_runtime_neutral_dependencies() {
    let metadata = metadata(
        &[
            (
                "baozi-core",
                "baozi-core",
                vec![dependency("futures-core", json!(null))],
            ),
            ("futures-core", "futures-core", vec![]),
        ],
        &["baozi-core"],
    );

    let report = check_metadata(&metadata).expect("valid resolved metadata");

    assert_eq!(report.production_packages_checked, 1);
    assert!(report.violations.is_empty(), "{report:#?}");
}

#[test]
fn runtime_admission_rejects_normal_build_and_target_specific_schedulers() {
    for (kind, target) in [
        (json!(null), json!(null)),
        (json!("build"), json!(null)),
        (json!(null), json!("cfg(target_os = \"linux\")")),
    ] {
        let mut scheduler = dependency("rayon", kind);
        scheduler["dep_kinds"][0]["target"] = target;
        let metadata = metadata(
            &[
                ("baozi-core", "baozi-core", vec![scheduler]),
                ("rayon", "rayon", vec![]),
            ],
            &["baozi-core"],
        );

        let report = check_metadata(&metadata).expect("valid resolved metadata");

        assert_eq!(report.violations.len(), 1, "{report:#?}");
        assert!(
            report.violations[0].ends_with("baozi-core -> rayon"),
            "{report:#?}"
        );
    }
}

#[test]
fn runtime_admission_follows_transitive_package_ids_despite_dependency_renames() {
    let scheduler_id = "registry+https://example.invalid/index#tokio@1.0.0";
    let mut scheduler = dependency(scheduler_id, json!(null));
    scheduler["name"] = json!("renamed_scheduler");
    let metadata = metadata(
        &[
            (
                "baozi-core",
                "baozi-core",
                vec![dependency("codec", json!(null))],
            ),
            ("codec", "codec", vec![scheduler]),
            (scheduler_id, "tokio", vec![]),
        ],
        &["baozi-core"],
    );

    let report = check_metadata(&metadata).expect("valid resolved metadata");

    assert_eq!(report.violations.len(), 1, "{report:#?}");
    assert!(
        report.violations[0].ends_with("baozi-core -> codec -> tokio"),
        "{report:#?}"
    );
}

#[test]
fn runtime_admission_excludes_dev_only_paths_and_repository_tooling_roots() {
    let metadata = metadata(
        &[
            (
                "baozi-core",
                "baozi-core",
                vec![dependency("codec", json!("dev"))],
            ),
            ("codec", "codec", vec![dependency("rayon", json!(null))]),
            (
                "baozi-test-support",
                "baozi-test-support",
                vec![dependency("rayon", json!(null))],
            ),
            (
                "baozi-xtask",
                "baozi-xtask",
                vec![dependency("rayon", json!(null))],
            ),
            ("rayon", "rayon", vec![]),
        ],
        &["baozi-core", "baozi-test-support", "baozi-xtask", "codec"],
    );

    let report = check_metadata(&metadata).expect("valid resolved metadata");

    assert_eq!(report.production_packages_checked, 1);
    assert!(report.violations.is_empty(), "{report:#?}");
}

#[test]
fn runtime_admission_keeps_normal_and_build_kinds_on_mixed_dev_edges() {
    for kind in [json!(null), json!("build")] {
        let mut scheduler = dependency("rayon", json!("dev"));
        scheduler["dep_kinds"]
            .as_array_mut()
            .expect("dependency kinds array")
            .push(json!({"kind": kind, "target": null}));
        let metadata = metadata(
            &[
                ("baozi-core", "baozi-core", vec![scheduler]),
                ("rayon", "rayon", vec![]),
            ],
            &["baozi-core"],
        );

        let report = check_metadata(&metadata).expect("valid resolved metadata");

        assert_eq!(report.violations.len(), 1, "{report:#?}");
    }
}

#[test]
fn runtime_admission_reports_each_production_root_and_terminates_on_cycles() {
    let metadata = metadata(
        &[
            (
                "baozi",
                "baozi",
                vec![dependency("baozi-core", json!(null))],
            ),
            (
                "baozi-core",
                "baozi-core",
                vec![dependency("codec", json!(null))],
            ),
            (
                "codec",
                "codec",
                vec![
                    dependency("baozi-core", json!(null)),
                    dependency("smol", json!(null)),
                ],
            ),
            ("smol", "smol", vec![]),
        ],
        &["baozi", "baozi-core"],
    );

    let report = check_metadata(&metadata).expect("valid resolved metadata");

    assert_eq!(report.production_packages_checked, 2);
    assert_eq!(report.violations.len(), 2, "{report:#?}");
    assert!(
        report.violations[0].ends_with("baozi -> baozi-core -> codec -> smol"),
        "{report:#?}"
    );
    assert!(
        report.violations[1].ends_with("baozi-core -> codec -> smol"),
        "{report:#?}"
    );
}

#[test]
fn runtime_admission_rejects_incomplete_resolve_graphs() {
    let valid = metadata(
        &[
            (
                "baozi-core",
                "baozi-core",
                vec![dependency("rayon", json!(null))],
            ),
            ("rayon", "rayon", vec![]),
        ],
        &["baozi-core"],
    );
    let mut missing_resolve = valid.clone();
    missing_resolve["resolve"] = Value::Null;
    let mut missing_node = valid.clone();
    missing_node["resolve"]["nodes"]
        .as_array_mut()
        .expect("resolve nodes")
        .pop();
    let mut missing_package = valid;
    missing_package["packages"]
        .as_array_mut()
        .expect("packages")
        .pop();

    for invalid in [missing_resolve, missing_node, missing_package] {
        assert!(check_metadata(&invalid).is_err(), "{invalid:#?}");
    }
}

#[test]
fn runtime_admission_rejects_missing_or_unknown_dependency_kinds() {
    for kinds in [
        json!([]),
        json!([{}]),
        json!([{"kind": "unknown"}]),
        Value::Null,
    ] {
        let mut scheduler = dependency("rayon", json!(null));
        scheduler["dep_kinds"] = kinds;
        let metadata = metadata(
            &[
                ("baozi-core", "baozi-core", vec![scheduler]),
                ("rayon", "rayon", vec![]),
            ],
            &["baozi-core"],
        );

        assert!(check_metadata(&metadata).is_err(), "{metadata:#?}");
    }
}

fn dependency(package: &str, kind: Value) -> Value {
    json!({
        "name": package,
        "pkg": package,
        "dep_kinds": [{"kind": kind, "target": null}],
    })
}

fn metadata(packages: &[(&str, &str, Vec<Value>)], workspace_members: &[&str]) -> Value {
    json!({
        "packages": packages.iter().map(|(id, name, _)| json!({"id": id, "name": name})).collect::<Vec<_>>(),
        "workspace_members": workspace_members,
        "resolve": {
            "nodes": packages.iter().map(|(id, _, deps)| json!({"id": id, "deps": deps})).collect::<Vec<_>>(),
        },
    })
}
