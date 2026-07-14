use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn format_ownership_recursively_scans_unlinked_rust_sources() {
    let repository = TestRepository::new("recursive-unlinked-source");
    repository.write_source("baozi-format-stl", "src/lib.rs", "pub struct Parser;");
    repository.write_source(
        "baozi-format-stl",
        "src/generated/orphan.rs",
        "struct MeteredStorage;",
    );
    repository.write_source(
        "baozi-format-stl",
        "tests/not-production.rs",
        "struct HeldReservation;",
    );
    repository.write_source(
        "unrelated-format",
        "src/not_first_party.rs",
        "struct LiveStorage;",
    );

    let report = baozi_xtask::format_ownership::check_repository(repository.path());

    assert_eq!(report.format_source_files_checked, 2);
    assert_eq!(report.violations.len(), 1, "{:#?}", report.violations);
    let violation = &report.violations[0];
    assert!(
        violation.contains("forbidden protocol \"MeteredStorage\"")
            && violation.contains("generated")
            && violation.contains("orphan.rs"),
        "{:#?}",
        report.violations
    );
}

#[test]
fn format_ownership_accepts_central_owning_wrappers() {
    let repository = TestRepository::new("central-owning-wrappers");
    repository.write_source(
        "baozi-format-obj",
        "src/nested/parser.rs",
        concat!(
            "use baozi_import::{MeteredVec, PreparedSceneValue};\n",
            "pub struct Parser;\n",
        ),
    );

    let report = baozi_xtask::format_ownership::check_repository(repository.path());

    assert_eq!(report.format_source_files_checked, 1);
    assert!(report.violations.is_empty(), "{:#?}", report.violations);
}

struct TestRepository {
    root: PathBuf,
}

impl TestRepository {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "baozi-format-ownership-{name}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("crates")).expect("create test repository");
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn write_source(&self, package: &str, relative_path: &str, source: &str) {
        let path = self.root.join("crates").join(package).join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create source parent");
        }
        fs::write(path, source).expect("write source fixture");
    }
}

impl Drop for TestRepository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
