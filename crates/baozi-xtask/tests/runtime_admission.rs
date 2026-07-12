use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn runtime_admission_accepts_runtime_neutral_production_sources() {
    let workspace = TestWorkspace::new("valid-runtime-neutrality");
    workspace.write_package(
        "baozi-core",
        "",
        r#"
use std::sync::{Arc, OnceLock};

struct Codec;

static CODEC: OnceLock<Arc<Codec>> = OnceLock::new();

fn parser_local_graph_walk() {
    let mut queue = Vec::new();
    queue.push(1_u8);
    let _ = queue.pop();
    let _example = "std::thread::spawn and Semaphore are documentation text";
    // std::thread::spawn is forbidden in production, but comments are not code.
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_threads_are_not_production_scheduling() {
        std::thread::spawn(|| {}).join().unwrap();
    }
}

#[cfg(test)]
mod external_test_helpers;
"#,
    );
    workspace.write_source(
        "baozi-core",
        "src/external_test_helpers.rs",
        "pub fn helper() { std::thread::spawn(|| {}); }",
    );
    workspace.write_source(
        "baozi-core",
        "tests/integration.rs",
        "#[test] fn host_thread() { std::thread::spawn(|| {}); }",
    );
    workspace.write_package(
        "baozi-xtask",
        "",
        "pub fn run_command() { let _ = std::process::Command::new(\"cargo\").spawn(); }",
    );

    let report = baozi_xtask::runtime_admission::check_repository(workspace.path());

    assert!(report.violations.is_empty(), "{:#?}", report.violations);
    assert_eq!(report.production_packages_checked, 1);
    assert_eq!(report.production_source_files_checked, 1);
}

#[test]
fn runtime_admission_rejects_normal_build_optional_and_transitive_schedulers() {
    for (fixture, manifest) in [
        ("normal", "rayon = { path = \"../rayon\" }"),
        (
            "build",
            "\n[build-dependencies]\nrayon = { path = \"../rayon\" }",
        ),
        (
            "optional",
            concat!(
                "rayon = { path = \"../rayon\", optional = true }\n\n",
                "[features]\nparallel = [\"dep:rayon\"]",
            ),
        ),
        (
            "transitive",
            "runtime-helper = { path = \"../runtime-helper\" }",
        ),
    ] {
        let workspace = TestWorkspace::new(&format!("forbidden-{fixture}-dependency"));
        workspace.write_package("baozi-core", manifest, "");
        workspace.write_package("rayon", "", "");
        workspace.write_package("runtime-helper", "rayon = { path = \"../rayon\" }", "");

        let report = baozi_xtask::runtime_admission::check_repository(workspace.path());

        assert!(
            report.violations.iter().any(|violation| {
                violation.contains("forbidden runtime dependency")
                    && violation.contains("baozi-core")
                    && violation.contains("rayon")
            }),
            "fixture {fixture}: {:#?}",
            report.violations
        );
    }
}

#[test]
fn runtime_admission_excludes_dev_only_scheduler_dependencies() {
    let workspace = TestWorkspace::new("allowed-dev-scheduler");
    workspace.write_package(
        "baozi-core",
        "\n[dev-dependencies]\nrayon = { path = \"../rayon\" }",
        "",
    );
    workspace.write_package("rayon", "", "");

    let report = baozi_xtask::runtime_admission::check_repository(workspace.path());

    assert!(report.violations.is_empty(), "{:#?}", report.violations);
}

#[test]
fn runtime_admission_rejects_explicit_production_thread_entry_points() {
    for (fixture, source) in [
        (
            "qualified-spawn",
            "pub fn run() { std::thread::spawn(|| {}); }",
        ),
        (
            "aliased-spawn",
            concat!(
                "use std::thread as worker_thread;\n",
                "pub fn run() { worker_thread::spawn(|| {}); }",
            ),
        ),
        (
            "scoped-threads",
            "pub fn run() { std::thread::scope(|scope| { scope.spawn(|| {}); }); }",
        ),
        (
            "builder-spawn",
            concat!(
                "pub fn run() {\n",
                "    std::thread::Builder::new().spawn(|| {}).unwrap();\n",
                "}\n",
            ),
        ),
        (
            "not-test-only",
            concat!(
                "#[cfg(any(test, feature = \"parallel\"))]\n",
                "pub fn run() { std::thread::spawn(|| {}); }\n",
            ),
        ),
    ] {
        let workspace = TestWorkspace::new(&format!("forbidden-{fixture}"));
        workspace.write_package_with_features("baozi-core", "", "parallel = []", source);

        let report = baozi_xtask::runtime_admission::check_repository(workspace.path());

        assert!(
            report.violations.iter().any(|violation| {
                violation.contains("production thread entry point")
                    && violation.contains("baozi-core")
            }),
            "fixture {fixture}: {:#?}",
            report.violations
        );
    }
}

#[test]
fn runtime_admission_rejects_global_semaphore_and_worker_pool_state() {
    for (fixture, source, type_name) in [
        (
            "semaphore",
            "struct Semaphore; static GLOBAL_GATE: Semaphore = Semaphore;",
            "Semaphore",
        ),
        (
            "thread-pool",
            "struct ThreadPool; static GLOBAL_POOL: ThreadPool = ThreadPool;",
            "ThreadPool",
        ),
    ] {
        let workspace = TestWorkspace::new(&format!("forbidden-global-{fixture}"));
        workspace.write_package("baozi-core", "", source);

        let report = baozi_xtask::runtime_admission::check_repository(workspace.path());

        assert!(
            report.violations.iter().any(|violation| {
                violation.contains("global admission or worker-pool state")
                    && violation.contains(type_name)
            }),
            "fixture {fixture}: {:#?}",
            report.violations
        );
    }
}

#[test]
fn runtime_admission_cli_checks_the_repository() {
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("xtask crate must be under the workspace root");
    let output = Command::new(env!("CARGO_BIN_EXE_baozi-xtask"))
        .arg("runtime-admission-check")
        .current_dir(repository_root)
        .output()
        .expect("run runtime admission check");

    assert_eq!(output.status.code(), Some(0), "{output:#?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Runtime admission check passed"),
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
        let root = std::env::temp_dir().join(format!(
            "baozi-runtime-admission-{name}-{}-{nonce}",
            std::process::id()
        ));
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

    fn write_package(&self, name: &str, dependencies: &str, source: &str) {
        self.write_package_with_features(name, dependencies, "", source);
    }

    fn write_package_with_features(
        &self,
        name: &str,
        dependencies: &str,
        features: &str,
        source: &str,
    ) {
        let directory = self.root.join("crates").join(name);
        fs::create_dir_all(directory.join("src")).expect("create package directory");
        let features = if features.is_empty() {
            String::new()
        } else {
            format!("\n[features]\n{features}\n")
        };
        fs::write(
            directory.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[dependencies]\n{dependencies}\n{features}"
            ),
        )
        .expect("write package manifest");
        fs::write(directory.join("src/lib.rs"), source).expect("write package target");
    }

    fn write_source(&self, package: &str, relative: &str, source: &str) {
        let path = self.root.join("crates").join(package).join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create source parent");
        }
        fs::write(path, source).expect("write source fixture");
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
