use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn operation_dependency_contract_accepts_required_edges_and_the_allowed_facade_edge() {
    let workspace = TestWorkspace::new("valid-operation-graph");
    workspace.write_canonical_graph();

    let report = baozi_xtask::crate_graph::check_repository(workspace.path());

    assert!(report.violations.is_empty(), "{:#?}", report.violations);
    assert_eq!(report.packages_checked, 7);
}

#[test]
fn operation_dependency_contract_rejects_upward_and_bypass_edges() {
    let workspace = TestWorkspace::new("invalid-operation-graph");
    workspace.write_canonical_graph();
    workspace.write_package("baozi-operation", "baozi-io = { path = \"../baozi-io\" }");
    workspace.write_package(
        "baozi-format-stl",
        concat!(
            "baozi-core = { path = \"../baozi-core\" }\n",
            "baozi-import = { path = \"../baozi-import\" }\n",
            "baozi-operation = { path = \"../baozi-operation\" }",
        ),
    );

    let report = baozi_xtask::crate_graph::check_repository(workspace.path());

    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.contains("baozi-operation must not depend on baozi-io")),
        "{:#?}",
        report.violations
    );
    assert!(
        report.violations.iter().any(|violation| {
            violation.contains("baozi-format-stl must not depend directly on baozi-operation")
        }),
        "{:#?}",
        report.violations
    );
}

#[test]
fn operation_dependency_contract_requires_each_owner_edge() {
    let workspace = TestWorkspace::new("missing-operation-edge");
    workspace.write_canonical_graph();
    workspace.write_package(
        "baozi-import",
        concat!(
            "baozi-core = { path = \"../baozi-core\" }\n",
            "baozi-io = { path = \"../baozi-io\" }",
        ),
    );

    let report = baozi_xtask::crate_graph::check_repository(workspace.path());

    assert!(
        report.violations.iter().any(|violation| {
            violation.contains("baozi-import must depend directly on baozi-operation")
        }),
        "{:#?}",
        report.violations
    );
}

#[test]
fn operation_dependency_contract_does_not_require_the_facade_edge() {
    let workspace = TestWorkspace::new("facade-without-operation-edge");
    workspace.write_canonical_graph();
    workspace.write_package(
        "baozi",
        concat!(
            "baozi-core = { path = \"../baozi-core\" }\n",
            "baozi-io = { path = \"../baozi-io\" }\n",
            "baozi-import = { path = \"../baozi-import\" }\n",
            "baozi-postprocess = { path = \"../baozi-postprocess\" }\n",
            "baozi-format-stl = { path = \"../baozi-format-stl\", optional = true }",
        ),
    );

    let report = baozi_xtask::crate_graph::check_repository(workspace.path());

    assert!(report.violations.is_empty(), "{:#?}", report.violations);
}

#[test]
fn operation_dependency_contract_still_requires_the_facade_package() {
    let workspace = TestWorkspace::new("missing-facade-package");
    workspace.write_canonical_graph();
    fs::remove_dir_all(workspace.path().join("crates/baozi")).expect("remove facade fixture");

    let report = baozi_xtask::crate_graph::check_repository(workspace.path());

    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation == "workspace is missing baozi"),
        "{:#?}",
        report.violations
    );
}

#[test]
fn operation_dependency_contract_rejects_conditional_format_edges() {
    let workspace = TestWorkspace::new("conditional-format-operation-edge");
    workspace.write_canonical_graph();
    workspace.write_package(
        "baozi-format-stl",
        concat!(
            "baozi-core = { path = \"../baozi-core\" }\n",
            "baozi-import = { path = \"../baozi-import\" }\n\n",
            "[target.'cfg(target_os = \"windows\")'.dependencies]\n",
            "baozi-operation = { path = \"../baozi-operation\", optional = true }",
        ),
    );

    let report = baozi_xtask::crate_graph::check_repository(workspace.path());

    assert!(
        report.violations.iter().any(|violation| {
            violation == "baozi-format-stl must not depend directly on baozi-operation"
        }),
        "{:#?}",
        report.violations
    );
}

#[test]
fn operation_dependency_contract_rejects_additional_conditional_owner_edges() {
    let workspace = TestWorkspace::new("duplicate-conditional-operation-edge");
    workspace.write_canonical_graph();
    workspace.write_package(
        "baozi-core",
        concat!(
            "baozi-operation = { path = \"../baozi-operation\" }\n\n",
            "[target.'cfg(target_os = \"windows\")'.dependencies]\n",
            "baozi-operation = { path = \"../baozi-operation\" }",
        ),
    );

    let report = baozi_xtask::crate_graph::check_repository(workspace.path());

    assert!(
        report.violations.iter().any(|violation| violation
            == "baozi-core must have exactly one non-optional unconditional production dependency on baozi-operation"),
        "{:#?}",
        report.violations
    );
}

#[test]
fn operation_dependency_contract_rejects_core_import_reachability() {
    let workspace = TestWorkspace::new("invalid-core-import-reachability");
    workspace.write_canonical_graph();
    workspace.write_package(
        "baozi-import",
        concat!(
            "baozi-io = { path = \"../baozi-io\" }\n",
            "baozi-operation = { path = \"../baozi-operation\" }",
        ),
    );
    workspace.write_package(
        "baozi-core",
        concat!(
            "baozi-operation = { path = \"../baozi-operation\" }\n",
            "baozi-import = { path = \"../baozi-import\" }",
        ),
    );
    let report = baozi_xtask::crate_graph::check_repository(workspace.path());

    assert!(
        report.violations.iter().any(|violation| violation
            == "baozi-core must not reach baozi-import through production dependencies: baozi-core -> baozi-import"),
        "{:#?}",
        report.violations
    );
}

#[test]
fn operation_dependency_contract_rejects_indirect_postprocess_import_reachability() {
    let workspace = TestWorkspace::new("invalid-indirect-postprocess-import-reachability");
    workspace.write_canonical_graph();
    workspace.write_package(
        "baozi-postprocess",
        concat!(
            "baozi-core = { path = \"../baozi-core\" }\n",
            "baozi-operation = { path = \"../baozi-operation\" }\n",
            "baozi-layer-bridge = { path = \"../baozi-layer-bridge\" }",
        ),
    );
    workspace.write_package(
        "baozi-layer-bridge",
        "baozi-import = { path = \"../baozi-import\" }",
    );

    let report = baozi_xtask::crate_graph::check_repository(workspace.path());

    assert!(
        report.violations.iter().any(|violation| violation
            == "baozi-postprocess must not reach baozi-import through production dependencies: baozi-postprocess -> baozi-layer-bridge -> baozi-import"),
        "{:#?}",
        report.violations
    );
}

#[test]
fn operation_dependency_contract_rejects_postprocess_optional_import_path() {
    let workspace = TestWorkspace::new("invalid-postprocess-import-reachability");
    workspace.write_canonical_graph();
    workspace.write_package(
        "baozi-postprocess",
        concat!(
            "baozi-core = { path = \"../baozi-core\" }\n",
            "baozi-operation = { path = \"../baozi-operation\" }\n",
            "baozi-layer-bridge = { path = \"../baozi-layer-bridge\", optional = true }",
        ),
    );
    workspace.write_package(
        "baozi-layer-bridge",
        "baozi-import = { path = \"../baozi-import\", optional = true }",
    );

    let report = baozi_xtask::crate_graph::check_repository(workspace.path());

    assert!(
        report.violations.iter().any(|violation| violation
            == "baozi-postprocess must not reach baozi-import through production dependencies: baozi-postprocess -> baozi-layer-bridge -> baozi-import"),
        "{:#?}",
        report.violations
    );
}

#[test]
fn operation_dependency_contract_rejects_target_specific_core_import_path() {
    let workspace = TestWorkspace::new("invalid-target-specific-core-import-reachability");
    workspace.write_canonical_graph();
    workspace.write_package(
        "baozi-core",
        concat!(
            "baozi-operation = { path = \"../baozi-operation\" }\n\n",
            "[target.'cfg(target_os = \"windows\")'.dependencies]\n",
            "baozi-layer-bridge = { path = \"../baozi-layer-bridge\" }",
        ),
    );
    workspace.write_package(
        "baozi-layer-bridge",
        "baozi-import = { path = \"../baozi-import\" }",
    );

    let report = baozi_xtask::crate_graph::check_repository(workspace.path());

    assert!(
        report.violations.iter().any(|violation| violation
            == "baozi-core must not reach baozi-import through production dependencies: baozi-core -> baozi-layer-bridge -> baozi-import"),
        "{:#?}",
        report.violations
    );
}

#[test]
fn operation_dependency_contract_includes_build_edges_and_excludes_dev_edges() {
    let build_workspace = TestWorkspace::new("invalid-build-core-import-reachability");
    build_workspace.write_canonical_graph();
    build_workspace.write_package(
        "baozi-core",
        concat!(
            "baozi-operation = { path = \"../baozi-operation\" }\n\n",
            "[build-dependencies]\n",
            "baozi-layer-bridge = { path = \"../baozi-layer-bridge\" }",
        ),
    );
    build_workspace.write_package(
        "baozi-layer-bridge",
        "baozi-import = { path = \"../baozi-import\" }",
    );

    let build_report = baozi_xtask::crate_graph::check_repository(build_workspace.path());
    assert!(
        build_report.violations.iter().any(|violation| violation
            == "baozi-core must not reach baozi-import through production dependencies: baozi-core -> baozi-layer-bridge -> baozi-import"),
        "{:#?}",
        build_report.violations
    );

    let dev_workspace = TestWorkspace::new("valid-dev-core-import-reachability");
    dev_workspace.write_canonical_graph();
    dev_workspace.write_package(
        "baozi-core",
        concat!(
            "baozi-operation = { path = \"../baozi-operation\" }\n\n",
            "[dev-dependencies]\n",
            "baozi-import = { path = \"../baozi-import\" }",
        ),
    );

    let dev_report = baozi_xtask::crate_graph::check_repository(dev_workspace.path());
    assert!(
        dev_report.violations.is_empty(),
        "{:#?}",
        dev_report.violations
    );
}

#[test]
fn dependency_direction_contract_rejects_every_forbidden_production_layer() {
    let cases = [
        (
            "core-to-io",
            "baozi-core",
            concat!(
                "baozi-operation = { path = \"../baozi-operation\" }\n",
                "baozi-io = { path = \"../baozi-io\" }",
            ),
            "baozi-core must not depend on baozi-io through production dependencies",
        ),
        (
            "io-to-core",
            "baozi-io",
            "baozi-core = { path = \"../baozi-core\" }",
            "baozi-io must not depend on baozi-core through production dependencies",
        ),
        (
            "import-to-postprocess",
            "baozi-import",
            concat!(
                "baozi-core = { path = \"../baozi-core\" }\n",
                "baozi-io = { path = \"../baozi-io\" }\n",
                "baozi-operation = { path = \"../baozi-operation\" }\n",
                "baozi-postprocess = { path = \"../baozi-postprocess\" }",
            ),
            "baozi-import must not depend on baozi-postprocess through production dependencies",
        ),
        (
            "postprocess-to-io",
            "baozi-postprocess",
            concat!(
                "baozi-core = { path = \"../baozi-core\" }\n",
                "baozi-operation = { path = \"../baozi-operation\" }\n",
                "baozi-io = { path = \"../baozi-io\" }",
            ),
            "baozi-postprocess must not depend on baozi-io through production dependencies",
        ),
        (
            "format-to-postprocess",
            "baozi-format-stl",
            concat!(
                "baozi-core = { path = \"../baozi-core\" }\n",
                "baozi-import = { path = \"../baozi-import\" }\n",
                "baozi-postprocess = { path = \"../baozi-postprocess\" }",
            ),
            "baozi-format-stl must not depend on baozi-postprocess through production dependencies",
        ),
    ];

    for (name, package, dependencies, expected) in cases {
        let workspace = TestWorkspace::new(name);
        workspace.write_canonical_graph();
        workspace.write_package(package, dependencies);

        let report = baozi_xtask::crate_graph::check_repository(workspace.path());

        assert!(
            report
                .violations
                .iter()
                .any(|violation| violation == expected),
            "{name}: {:#?}",
            report.violations
        );
    }
}

#[test]
fn dependency_direction_contract_rejects_production_edges_to_support_tools() {
    let workspace = TestWorkspace::new("production-to-support-tool");
    workspace.write_canonical_graph();
    workspace.write_package("baozi-test-support", "");
    workspace.write_package("baozi-xtask", "");
    workspace.write_package(
        "baozi-core",
        concat!(
            "baozi-operation = { path = \"../baozi-operation\" }\n",
            "baozi-test-support = { path = \"../baozi-test-support\" }",
        ),
    );
    workspace.write_package(
        "baozi-format-stl",
        concat!(
            "baozi-core = { path = \"../baozi-core\" }\n",
            "baozi-import = { path = \"../baozi-import\" }\n",
            "baozi-xtask = { path = \"../baozi-xtask\" }",
        ),
    );

    let report = baozi_xtask::crate_graph::check_repository(workspace.path());

    for expected in [
        "baozi-core must not depend on non-production workspace package baozi-test-support through production dependencies",
        "baozi-format-stl must not depend on non-production workspace package baozi-xtask through production dependencies",
    ] {
        assert!(
            report
                .violations
                .iter()
                .any(|violation| violation == expected),
            "{:#?}",
            report.violations
        );
    }
}

#[test]
fn dependency_direction_contract_rejects_unclassified_production_crates() {
    let workspace = TestWorkspace::new("unclassified-production-crate");
    workspace.write_canonical_graph();
    workspace.write_package("baozi-cli", "");

    let report = baozi_xtask::crate_graph::check_repository(workspace.path());

    assert!(
        report.violations.iter().any(|violation| {
            violation == "baozi-cli has no production dependency-direction classification"
        }),
        "{:#?}",
        report.violations
    );
}

#[test]
fn dependency_direction_contract_checks_optional_target_build_and_renamed_edges() {
    let cases = [
        (
            "optional-postprocess-io",
            "baozi-postprocess",
            concat!(
                "baozi-core = { path = \"../baozi-core\" }\n",
                "baozi-operation = { path = \"../baozi-operation\" }\n",
                "baozi-io = { path = \"../baozi-io\", optional = true }",
            ),
            "baozi-postprocess must not depend on baozi-io through production dependencies",
        ),
        (
            "target-io-core",
            "baozi-io",
            concat!(
                "\n[target.'cfg(target_os = \"windows\")'.dependencies]\n",
                "baozi-core = { path = \"../baozi-core\" }",
            ),
            "baozi-io must not depend on baozi-core through production dependencies",
        ),
        (
            "build-format-postprocess",
            "baozi-format-stl",
            concat!(
                "baozi-core = { path = \"../baozi-core\" }\n",
                "baozi-import = { path = \"../baozi-import\" }\n\n",
                "[build-dependencies]\n",
                "baozi-postprocess = { path = \"../baozi-postprocess\" }",
            ),
            "baozi-format-stl must not depend on baozi-postprocess through production dependencies",
        ),
        (
            "renamed-io-core",
            "baozi-io",
            "domain = { package = \"baozi-core\", path = \"../baozi-core\" }",
            "baozi-io must not depend on baozi-core through production dependencies",
        ),
    ];

    for (name, package, dependencies, expected) in cases {
        let workspace = TestWorkspace::new(name);
        workspace.write_canonical_graph();
        workspace.write_package(package, dependencies);

        let report = baozi_xtask::crate_graph::check_repository(workspace.path());

        assert!(
            report
                .violations
                .iter()
                .any(|violation| violation == expected),
            "{name}: {:#?}",
            report.violations
        );
    }
}

#[test]
fn checked_in_workspace_passes_the_cli_gate() {
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("xtask crate must be under the workspace root")
        .to_path_buf();
    let output = Command::new(env!("CARGO_BIN_EXE_baozi-xtask"))
        .arg("crate-graph-check")
        .current_dir(repository_root)
        .output()
        .expect("run crate graph check");

    assert_eq!(output.status.code(), Some(0), "{output:#?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Crate graph check passed"),
        "{output:#?}"
    );
}

struct TestWorkspace {
    root: PathBuf,
}

impl TestWorkspace {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after Unix epoch")
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("baozi-xtask-{name}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(root.join("crates")).expect("create test workspace");
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nresolver = \"3\"\nmembers = [\"crates/*\"]\n",
        )
        .expect("write workspace manifest");
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn write_canonical_graph(&self) {
        self.write_package("baozi-operation", "");
        self.write_package(
            "baozi-core",
            "baozi-operation = { path = \"../baozi-operation\" }",
        );
        self.write_package("baozi-io", "");
        self.write_package(
            "baozi-import",
            concat!(
                "baozi-core = { path = \"../baozi-core\" }\n",
                "baozi-io = { path = \"../baozi-io\" }\n",
                "baozi-operation = { path = \"../baozi-operation\" }",
            ),
        );
        self.write_package(
            "baozi-postprocess",
            concat!(
                "baozi-core = { path = \"../baozi-core\" }\n",
                "baozi-operation = { path = \"../baozi-operation\" }",
            ),
        );
        self.write_package(
            "baozi-format-stl",
            concat!(
                "baozi-core = { path = \"../baozi-core\" }\n",
                "baozi-import = { path = \"../baozi-import\" }",
            ),
        );
        self.write_package(
            "baozi",
            concat!(
                "baozi-core = { path = \"../baozi-core\" }\n",
                "baozi-io = { path = \"../baozi-io\" }\n",
                "baozi-import = { path = \"../baozi-import\" }\n",
                "baozi-operation = { path = \"../baozi-operation\" }\n",
                "baozi-postprocess = { path = \"../baozi-postprocess\" }\n",
                "baozi-format-stl = { path = \"../baozi-format-stl\", optional = true }",
            ),
        );
    }

    fn write_package(&self, name: &str, dependencies: &str) {
        let directory = self.root.join("crates").join(name);
        fs::create_dir_all(directory.join("src")).expect("create package directory");
        fs::write(
            directory.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[dependencies]\n{dependencies}\n"
            ),
        )
        .expect("write package manifest");
        fs::write(directory.join("src/lib.rs"), "").expect("write package target");
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
