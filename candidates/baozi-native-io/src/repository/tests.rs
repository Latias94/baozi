use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "baozi-repository-{name}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("repository fixture root should be created");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn limits(spec: RepositoryLimitSpec) -> RepositoryLimits {
    spec.validate().expect("test limits should be valid")
}

fn generous_limits() -> RepositoryLimits {
    limits(RepositoryLimitSpec {
        max_roots: 16,
        max_entries: 256,
        max_files: 128,
        max_directories: 128,
        max_depth: 64,
        max_path_bytes: 4096,
        max_file_bytes: 1_048_576,
        max_total_bytes: 4_194_304,
        max_work: 1_000_000,
    })
}

fn file_root(path: &str) -> RepositoryRootSpec {
    RepositoryRootSpec::File(RepositoryPath::new(path).expect("valid file root"))
}

fn tree_root(path: &str) -> RepositoryRootSpec {
    RepositoryRootSpec::Tree(RepositoryPath::new(path).expect("valid tree root"))
}

#[test]
fn repository_path_preserves_spelling_and_rejects_non_portable_forms() {
    let path = RepositoryPath::new("Source/MODEL_01.rs").expect("portable path");
    assert_eq!(path.as_str(), "Source/MODEL_01.rs");
    assert_eq!(path.component_count(), 2);
    assert_eq!(path.file_name(), "MODEL_01.rs");
    assert_eq!(path.try_clone().expect("fallible clone"), path);

    let cases = [
        ("", RepositoryPathErrorCode::Empty),
        ("/absolute", RepositoryPathErrorCode::Absolute),
        ("a\\b", RepositoryPathErrorCode::Backslash),
        ("a//b", RepositoryPathErrorCode::EmptyComponent),
        ("a/./b", RepositoryPathErrorCode::DotComponent),
        ("a/../b", RepositoryPathErrorCode::DotComponent),
        (" leading", RepositoryPathErrorCode::LeadingOrTrailingSpace),
        ("trailing ", RepositoryPathErrorCode::LeadingOrTrailingSpace),
        ("trailing.", RepositoryPathErrorCode::TrailingDot),
        ("line\nfeed", RepositoryPathErrorCode::ControlCharacter),
        ("caf\u{00e9}", RepositoryPathErrorCode::NonPortableUnicode),
        ("file:name", RepositoryPathErrorCode::InvalidCharacter),
        ("PROGRA~1/file", RepositoryPathErrorCode::ShortNameAlias),
        ("NUL.txt", RepositoryPathErrorCode::ReservedDevice),
    ];
    for (candidate, code) in cases {
        assert_eq!(
            RepositoryPath::new(candidate)
                .expect_err("non-portable path must fail")
                .code(),
            code,
            "{candidate:?}"
        );
    }

    let component = "a".repeat(RepositoryPath::HARD_MAX_COMPONENT_BYTES + 1);
    assert_eq!(
        RepositoryPath::new(component)
            .expect_err("overlong component must fail")
            .code(),
        RepositoryPathErrorCode::ComponentTooLong
    );
    let overlong = format!("{}a", "a/".repeat(RepositoryPath::HARD_MAX_BYTES / 2));
    assert_eq!(overlong.len(), RepositoryPath::HARD_MAX_BYTES + 1);
    assert_eq!(
        RepositoryPath::new(overlong)
            .expect_err("overlong path must fail")
            .code(),
        RepositoryPathErrorCode::TooLong
    );
}

#[test]
fn repository_limits_accept_exact_boundaries_and_reject_invalid_specs() {
    let baseline = RepositoryLimitSpec {
        max_roots: 1,
        max_entries: 1,
        max_files: 1,
        max_directories: 1,
        max_depth: 1,
        max_path_bytes: 1,
        max_file_bytes: 1,
        max_total_bytes: 1,
        max_work: 1,
    };
    let exact = RepositoryLimitSpec {
        max_path_bytes: 4096,
        ..baseline
    }
    .validate()
    .expect("hard path maximum is admitted");
    assert_eq!(exact.max_path_bytes(), 4096);

    for (limit, spec) in [
        (
            RepositoryLimit::Roots,
            RepositoryLimitSpec {
                max_roots: 0,
                ..baseline
            },
        ),
        (
            RepositoryLimit::Entries,
            RepositoryLimitSpec {
                max_entries: 0,
                ..baseline
            },
        ),
        (
            RepositoryLimit::Files,
            RepositoryLimitSpec {
                max_files: 0,
                ..baseline
            },
        ),
        (
            RepositoryLimit::Directories,
            RepositoryLimitSpec {
                max_directories: 0,
                ..baseline
            },
        ),
        (
            RepositoryLimit::Depth,
            RepositoryLimitSpec {
                max_depth: 0,
                ..baseline
            },
        ),
        (
            RepositoryLimit::PathBytes,
            RepositoryLimitSpec {
                max_path_bytes: 0,
                ..baseline
            },
        ),
        (
            RepositoryLimit::FileBytes,
            RepositoryLimitSpec {
                max_file_bytes: 0,
                ..baseline
            },
        ),
        (
            RepositoryLimit::TotalBytes,
            RepositoryLimitSpec {
                max_total_bytes: 0,
                ..baseline
            },
        ),
        (
            RepositoryLimit::Work,
            RepositoryLimitSpec {
                max_work: 0,
                ..baseline
            },
        ),
    ] {
        assert_eq!(spec.validate(), Err(RepositoryLimitsError::Zero { limit }));
    }

    assert_eq!(
        RepositoryLimitSpec {
            max_path_bytes: 4097,
            ..baseline
        }
        .validate(),
        Err(RepositoryLimitsError::PathExceedsHardMaximum {
            actual: 4097,
            hard_maximum: 4096,
        })
    );
    assert_eq!(
        RepositoryLimitSpec {
            max_files: u64::MAX,
            ..baseline
        }
        .validate(),
        Err(RepositoryLimitsError::ArithmeticOverflow)
    );
}

#[test]
fn capture_errors_discard_backend_sources_and_messages() {
    let path = RepositoryPath::new("repository/input.bin").expect("portable repository path");
    let error = RepositoryCaptureError::from_io(
        RepositoryCaptureStage::OpenFile,
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "sensitive backend detail",
        ),
        Some(&path),
    );
    assert_eq!(error.stage(), RepositoryCaptureStage::OpenFile);
    assert_eq!(error.kind(), RepositoryCaptureErrorKind::PermissionDenied);
    assert_eq!(error.path(), Some(&path));
    assert!(std::error::Error::source(&error).is_none());
    assert!(!format!("{error:?}").contains("sensitive backend detail"));
    assert!(!error.to_string().contains("sensitive backend detail"));
}

#[test]
fn reenumeration_scratch_is_bounded_by_original_entry_count() {
    let path = RepositoryPath::new("repository").expect("portable repository path");
    assert!(super::walk::ensure_reenumeration_scratch_capacity(1, 2, &path).is_ok());
    let error = super::walk::ensure_reenumeration_scratch_capacity(2, 2, &path)
        .expect_err("the first added entry must fail before scratch retention");
    assert_eq!(error.stage(), RepositoryCaptureStage::ReenumerateDirectory);
    assert_eq!(
        error.kind(),
        RepositoryCaptureErrorKind::DirectoryEntriesChanged
    );
    assert_eq!(error.path(), Some(&path));
}

#[test]
fn endpoint_revalidation_rejects_child_and_root_drift() {
    use super::native::ObjectIdentity;
    use super::walk::{EndpointObservation, compare_endpoint_observations};

    let expected_file = EndpointObservation::new(
        RepositoryObjectKind::File,
        ObjectIdentity {
            device: 11,
            object: 17,
        },
    );
    let child_path = RepositoryPath::new("root/child.bin").expect("portable child path");
    let child_error = compare_endpoint_observations(
        expected_file,
        EndpointObservation::new(
            RepositoryObjectKind::File,
            ObjectIdentity {
                device: 11,
                object: 19,
            },
        ),
        RepositoryCaptureStage::ReenumerateDirectory,
        &child_path,
    )
    .expect_err("child identity drift must fail closed");
    assert_eq!(
        child_error.stage(),
        RepositoryCaptureStage::ReenumerateDirectory
    );
    assert_eq!(
        child_error.kind(),
        RepositoryCaptureErrorKind::ObjectChangedDuringRead
    );
    assert_eq!(child_error.path(), Some(&child_path));

    let root_path = RepositoryPath::new("root").expect("portable root path");
    let root_error = compare_endpoint_observations(
        expected_file,
        EndpointObservation::new(
            RepositoryObjectKind::Directory,
            ObjectIdentity {
                device: 11,
                object: 17,
            },
        ),
        RepositoryCaptureStage::Finalize,
        &root_path,
    )
    .expect_err("root kind drift must fail closed");
    assert_eq!(root_error.stage(), RepositoryCaptureStage::Finalize);
    assert_eq!(
        root_error.kind(),
        RepositoryCaptureErrorKind::ObjectChangedDuringRead
    );
    assert_eq!(root_error.path(), Some(&root_path));

    compare_endpoint_observations(
        expected_file,
        expected_file,
        RepositoryCaptureStage::Finalize,
        &root_path,
    )
    .expect("equal endpoint observations must pass");
}

#[test]
fn final_content_revalidation_rejects_same_identity_byte_drift() {
    use super::native::ObjectIdentity;
    use super::walk::{
        EndpointObservation, compare_endpoint_observations, compare_final_file_bytes,
    };

    let path = RepositoryPath::new("repository/input.bin").expect("portable repository path");
    let original = EndpointObservation::new(
        RepositoryObjectKind::File,
        ObjectIdentity {
            device: 23,
            object: 29,
        },
    );
    compare_endpoint_observations(original, original, RepositoryCaptureStage::Finalize, &path)
        .expect("same inode and kind alone cannot detect content drift");

    let error = compare_final_file_bytes(b"same-length-A", b"same-length-B", &path)
        .expect_err("final content pass must reject same-inode byte drift");
    assert_eq!(error.stage(), RepositoryCaptureStage::Finalize);
    assert_eq!(
        error.kind(),
        RepositoryCaptureErrorKind::ObjectChangedDuringRead
    );
    assert_eq!(error.path(), Some(&path));
}

#[test]
fn final_directory_snapshot_rejects_same_identity_entry_drift() {
    use super::walk::compare_directory_snapshot_names;

    let path = RepositoryPath::new("repository").expect("portable repository path");
    let expected = vec!["stable.bin".to_owned()];
    let error = compare_directory_snapshot_names(
        &expected,
        ["stable.bin", "added.bin"].into_iter(),
        RepositoryCaptureStage::Finalize,
        &path,
    )
    .expect_err("final directory snapshot must reject an added entry");
    assert_eq!(error.stage(), RepositoryCaptureStage::Finalize);
    assert_eq!(
        error.kind(),
        RepositoryCaptureErrorKind::DirectoryEntriesChanged
    );
    assert_eq!(error.path(), Some(&path));
}

#[cfg(unix)]
#[test]
fn repository_resource_limits_accept_exact_and_reject_one_over() {
    let fixture = TempRoot::new("resource-limits");
    fs::create_dir_all(fixture.path().join("repo/nested")).expect("create nested directory");
    fs::create_dir_all(fixture.path().join("repo/empty")).expect("create empty directory");
    fs::write(fixture.path().join("repo/a.bin"), b"ab").expect("write first file");
    fs::write(fixture.path().join("repo/nested/b.bin"), b"xyz").expect("write second file");
    let repository = RepositoryRoot::open_ambient(fixture.path()).expect("open repository root");

    let baseline = repository
        .capture(vec![tree_root("repo")], generous_limits())
        .expect("generous capture");
    let usage = baseline.usage();
    assert_eq!(usage.total_bytes(), 5);
    let exact_spec = RepositoryLimitSpec {
        max_roots: usage.roots(),
        max_entries: usage.entries(),
        max_files: usage.files(),
        max_directories: usage.directories(),
        max_depth: usage.max_depth(),
        max_path_bytes: usage.max_path_bytes(),
        max_file_bytes: usage.max_file_bytes(),
        max_total_bytes: usage.total_bytes(),
        max_work: usage.work(),
    };
    repository
        .capture(
            vec![tree_root("repo")],
            exact_spec.validate().expect("exact limits"),
        )
        .expect("every exact boundary should succeed");

    let mut spec = exact_spec;
    spec.max_entries -= 1;
    assert_limit(
        repository.capture(
            vec![tree_root("repo")],
            spec.validate().expect("entry limit"),
        ),
        RepositoryLimit::Entries,
    );

    let mut spec = exact_spec;
    spec.max_files -= 1;
    assert_limit(
        repository.capture(
            vec![tree_root("repo")],
            spec.validate().expect("file limit"),
        ),
        RepositoryLimit::Files,
    );

    let mut spec = exact_spec;
    spec.max_directories -= 1;
    assert_limit(
        repository.capture(
            vec![tree_root("repo")],
            spec.validate().expect("directory limit"),
        ),
        RepositoryLimit::Directories,
    );

    let mut spec = exact_spec;
    spec.max_depth -= 1;
    assert_limit(
        repository.capture(
            vec![tree_root("repo")],
            spec.validate().expect("depth limit"),
        ),
        RepositoryLimit::Depth,
    );

    let mut spec = exact_spec;
    spec.max_path_bytes -= 1;
    assert_limit(
        repository.capture(
            vec![tree_root("repo")],
            spec.validate().expect("path limit"),
        ),
        RepositoryLimit::PathBytes,
    );

    let mut spec = exact_spec;
    spec.max_file_bytes -= 1;
    assert_limit(
        repository.capture(
            vec![tree_root("repo")],
            spec.validate().expect("file bytes"),
        ),
        RepositoryLimit::FileBytes,
    );

    let mut spec = exact_spec;
    spec.max_total_bytes -= 1;
    assert_limit(
        repository.capture(
            vec![tree_root("repo")],
            spec.validate().expect("total bytes"),
        ),
        RepositoryLimit::TotalBytes,
    );

    let mut spec = exact_spec;
    spec.max_work -= 1;
    assert_limit(
        repository.capture(
            vec![tree_root("repo")],
            spec.validate().expect("work limit"),
        ),
        RepositoryLimit::Work,
    );

    let root_exact = limits(RepositoryLimitSpec {
        max_roots: 2,
        max_entries: 2,
        max_files: 2,
        max_directories: 1,
        max_depth: 8,
        max_path_bytes: 128,
        max_file_bytes: 8,
        max_total_bytes: 8,
        max_work: 10_000,
    });
    repository
        .capture(
            vec![file_root("repo/a.bin"), file_root("repo/nested/b.bin")],
            root_exact,
        )
        .expect("exact root count should succeed");
    let root_one_over = limits(RepositoryLimitSpec {
        max_roots: 1,
        max_entries: 2,
        max_files: 2,
        max_directories: 1,
        max_depth: 8,
        max_path_bytes: 128,
        max_file_bytes: 8,
        max_total_bytes: 8,
        max_work: 10_000,
    });
    assert_limit(
        repository.capture(
            vec![file_root("repo/a.bin"), file_root("repo/nested/b.bin")],
            root_one_over,
        ),
        RepositoryLimit::Roots,
    );
}

#[cfg(unix)]
#[test]
fn deterministic_capture_records_files_and_empty_directories() {
    let fixture = TempRoot::new("deterministic");
    fs::create_dir_all(fixture.path().join("tree/empty")).expect("create empty directory");
    fs::write(fixture.path().join("tree/z.bin"), b"z").expect("write z");
    fs::write(fixture.path().join("tree/a.bin"), b"a").expect("write a");
    fs::write(fixture.path().join("standalone.bin"), b"s").expect("write standalone");
    let repository = RepositoryRoot::open_ambient(fixture.path()).expect("open repository root");

    let capture = repository
        .capture(
            vec![tree_root("tree"), file_root("standalone.bin")],
            generous_limits(),
        )
        .expect("capture repository");

    let roots: Vec<_> = capture
        .roots()
        .iter()
        .map(|root| root.path().as_str())
        .collect();
    assert_eq!(roots, ["standalone.bin", "tree"]);
    let paths: Vec<_> = capture
        .files()
        .iter()
        .map(|file| file.path().as_str())
        .collect();
    assert_eq!(paths, ["standalone.bin", "tree/a.bin", "tree/z.bin"]);
    assert_eq!(capture.usage().entries(), 5);
    assert_eq!(capture.usage().directories(), 2);
    repository
        .verify_unchanged(&capture)
        .expect("complete recapture should match");
}

#[test]
fn rejects_duplicate_overlapping_and_case_alias_roots() {
    let fixture = TempRoot::new("root-preflight");
    let repository = RepositoryRoot::open_ambient(fixture.path()).expect("open repository root");

    let duplicate = repository
        .capture(
            vec![file_root("same"), tree_root("same")],
            generous_limits(),
        )
        .expect_err("duplicate root must fail before IO");
    assert_eq!(duplicate.kind(), RepositoryCaptureErrorKind::DuplicateRoot);

    let overlapping = repository
        .capture(
            vec![tree_root("repo"), file_root("repo/file")],
            generous_limits(),
        )
        .expect_err("overlapping roots must fail before IO");
    assert_eq!(
        overlapping.kind(),
        RepositoryCaptureErrorKind::OverlappingRoots
    );

    let alias = repository
        .capture(
            vec![file_root("Source"), file_root("source")],
            generous_limits(),
        )
        .expect_err("portable case alias must fail before IO");
    assert_eq!(
        alias.kind(),
        RepositoryCaptureErrorKind::PortableNameCollision
    );
}

#[cfg(unix)]
#[test]
fn rejects_hard_link_aliases() {
    let fixture = TempRoot::new("hard-link");
    fs::create_dir(fixture.path().join("repo")).expect("create repository directory");
    fs::write(fixture.path().join("repo/original.bin"), b"payload").expect("write original");
    fs::hard_link(
        fixture.path().join("repo/original.bin"),
        fixture.path().join("repo/alias.bin"),
    )
    .expect("candidate runner must create a hard-link fixture");
    let repository = RepositoryRoot::open_ambient(fixture.path()).expect("open repository root");

    let error = repository
        .capture(vec![tree_root("repo")], generous_limits())
        .expect_err("hard-linked regular files must fail");
    assert!(matches!(
        error.kind(),
        RepositoryCaptureErrorKind::HardLinkedFile { links: 2 }
            | RepositoryCaptureErrorKind::IdentityAlias
    ));
}

#[cfg(unix)]
#[test]
fn verify_detects_added_and_removed_paths() {
    let fixture = TempRoot::new("verify-closure");
    fs::create_dir(fixture.path().join("repo")).expect("create repository directory");
    fs::write(fixture.path().join("repo/original.bin"), b"payload").expect("write original");
    let repository = RepositoryRoot::open_ambient(fixture.path()).expect("open repository root");
    let baseline = repository
        .capture(vec![tree_root("repo")], generous_limits())
        .expect("baseline capture");

    fs::create_dir(fixture.path().join("repo/zz-added")).expect("add empty directory");
    let added = repository
        .verify_unchanged(&baseline)
        .expect_err("added directory must be detected");
    assert!(matches!(
        added,
        RepositoryVerifyError::Added { ref path } if path.as_str() == "repo/zz-added"
    ));

    fs::remove_dir(fixture.path().join("repo/zz-added")).expect("remove added directory");
    repository
        .verify_unchanged(&baseline)
        .expect("restored closure should match");
    fs::remove_file(fixture.path().join("repo/original.bin")).expect("remove original");
    let removed = repository
        .verify_unchanged(&baseline)
        .expect_err("removed file must be detected");
    assert!(matches!(
        removed,
        RepositoryVerifyError::Removed { ref path } if path.as_str() == "repo/original.bin"
    ));

    fs::remove_dir(fixture.path().join("repo")).expect("remove explicit tree root");
    let removed_root = repository
        .verify_unchanged(&baseline)
        .expect_err("removed explicit root must be detected");
    assert!(matches!(
        removed_root,
        RepositoryVerifyError::Removed { ref path } if path.as_str() == "repo"
    ));
}

#[cfg(unix)]
#[test]
fn verify_detects_content_and_same_content_replacement() {
    let fixture = TempRoot::new("verify-content");
    fs::create_dir(fixture.path().join("repo")).expect("create repository directory");
    let source = fixture.path().join("repo/input.bin");
    fs::write(&source, b"first").expect("write original");
    let repository = RepositoryRoot::open_ambient(fixture.path()).expect("open repository root");
    let baseline = repository
        .capture(vec![tree_root("repo")], generous_limits())
        .expect("baseline capture");

    fs::write(&source, b"other").expect("change same file");
    let changed = repository
        .verify_unchanged(&baseline)
        .expect_err("raw bytes must be compared");
    assert!(matches!(
        changed,
        RepositoryVerifyError::ContentChanged { ref path }
            if path.as_str() == "repo/input.bin"
    ));

    fs::write(&source, b"first").expect("restore original bytes");
    repository
        .verify_unchanged(&baseline)
        .expect("same handle identity and bytes should match");
    fs::remove_file(&source).expect("unlink original while the capture retains its handle");
    let retained_metadata = baseline.files[0]
        ._handle
        .metadata()
        .expect("inspect the capture-retained file handle");
    assert_eq!(cap_fs_ext::MetadataExt::nlink(&retained_metadata), 0);
    fs::write(&source, b"first").expect("write same-content replacement");
    let replaced = repository
        .verify_unchanged(&baseline)
        .expect_err("same-content replacement must be detected");
    assert!(matches!(
        replaced,
        RepositoryVerifyError::Replaced { ref path } if path.as_str() == "repo/input.bin"
    ));
}

#[cfg(unix)]
#[test]
fn verify_detects_kind_changes() {
    let fixture = TempRoot::new("verify-kind");
    fs::create_dir(fixture.path().join("repo")).expect("create repository directory");
    let node = fixture.path().join("repo/node");
    fs::write(&node, b"file").expect("write file");
    let repository = RepositoryRoot::open_ambient(fixture.path()).expect("open repository root");
    let baseline = repository
        .capture(vec![tree_root("repo")], generous_limits())
        .expect("baseline capture");

    fs::remove_file(&node).expect("remove file");
    fs::create_dir(&node).expect("replace file with directory");
    let changed = repository
        .verify_unchanged(&baseline)
        .expect_err("kind drift must be detected");
    assert!(matches!(
        changed,
        RepositoryVerifyError::KindChanged {
            ref path,
            expected: RepositoryObjectKind::File,
            actual: RepositoryObjectKind::Directory,
        } if path.as_str() == "repo/node"
    ));
}

#[cfg(unix)]
#[test]
fn retained_repository_capability_ignores_textual_root_replacement() {
    let fixture = TempRoot::new("retained-root");
    let root_path = fixture.path().join("root");
    let moved_path = fixture.path().join("moved-root");
    fs::create_dir_all(root_path.join("scope")).expect("create retained scope");
    fs::write(root_path.join("scope/value.bin"), b"authorized").expect("write original");
    let repository = RepositoryRoot::open_ambient(&root_path).expect("open retained root");

    fs::rename(&root_path, &moved_path).expect("move retained root");
    fs::create_dir_all(root_path.join("scope")).expect("create replacement scope");
    fs::write(root_path.join("scope/value.bin"), b"replacement").expect("write replacement");

    let capture = repository
        .capture(vec![tree_root("scope")], generous_limits())
        .expect("capture through retained capability");
    assert_eq!(capture.files()[0].bytes(), b"authorized");
}

#[cfg(unix)]
#[test]
fn rejects_symlinks_in_roots_and_descendants() {
    use std::os::unix::fs::symlink;

    let fixture = TempRoot::new("symlink");
    fs::create_dir_all(fixture.path().join("real")).expect("create real directory");
    fs::write(fixture.path().join("real/value.bin"), b"value").expect("write real file");
    symlink("real", fixture.path().join("linked"))
        .expect("candidate runner must create a directory symlink fixture");
    let repository = RepositoryRoot::open_ambient(fixture.path()).expect("open repository root");
    assert!(
        repository
            .capture(vec![tree_root("linked")], generous_limits())
            .is_err(),
        "symlink root component must fail"
    );

    fs::create_dir(fixture.path().join("repo")).expect("create repository directory");
    symlink("../real/value.bin", fixture.path().join("repo/link.bin"))
        .expect("candidate runner must create a file symlink fixture");
    let descendant = repository
        .capture(vec![tree_root("repo")], generous_limits())
        .expect_err("symlink descendant must fail");
    assert!(matches!(
        descendant.stage(),
        RepositoryCaptureStage::OpenFile | RepositoryCaptureStage::InspectFile
    ));
}

#[cfg(unix)]
#[test]
fn rejects_trailing_slash_symlink_targets_during_root_and_child_traversal() {
    use std::os::unix::fs::symlink;

    let fixture = TempRoot::new("symlink-trailing-target");
    let repository_path = fixture.path().join("repo");
    let outside = fixture.path().join("outside");
    fs::create_dir_all(repository_path.join("scope")).expect("create repository scope");
    fs::create_dir_all(outside.join("nested")).expect("create outside directories");
    fs::write(outside.join("nested/secret.bin"), b"secret").expect("write outside file");
    let outside_target = format!("{}/", outside.display());
    symlink(&outside_target, fixture.path().join("root-escape"))
        .expect("create trailing-slash root symlink");
    symlink(&outside_target, repository_path.join("scope/escape"))
        .expect("create trailing-slash descendant symlink");

    let repository = RepositoryRoot::open_ambient(fixture.path()).expect("open repository root");
    assert!(
        repository
            .capture(vec![tree_root("root-escape")], generous_limits())
            .is_err(),
        "trailing slash must not make a symlink root component followable"
    );
    assert!(
        repository
            .capture(vec![tree_root("repo")], generous_limits())
            .is_err(),
        "trailing slash must not let a descendant symlink leave its declared tree"
    );
}

#[cfg(unix)]
#[test]
fn unix_special_files_fail_closed_without_blocking() {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::DirBuilderExt;
    use std::os::unix::net::UnixListener;
    use std::process::Command;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    // macOS's default temp directory can make Unix socket paths exceed sun_path.
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after the Unix epoch")
        .as_nanos();
    let path = Path::new("/tmp").join(format!("baozi-special-{}-{nonce:x}", std::process::id()));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .expect("create exclusive special-file fixture root");
    let fixture = TempRoot { path };
    fs::create_dir(fixture.path().join("fifo-root")).expect("create FIFO tree root");
    let fifo_path = fixture.path().join("fifo-root/input.fifo");
    let status = Command::new("mkfifo")
        .arg(&fifo_path)
        .status()
        .expect("Unix candidate runner must provide mkfifo");
    assert!(status.success(), "mkfifo failed with {status}");

    let fifo_repository =
        RepositoryRoot::open_ambient(fixture.path()).expect("open FIFO repository root");
    let (sender, receiver) = mpsc::channel();
    let fifo_worker = thread::spawn(move || {
        let _ =
            sender.send(fifo_repository.capture(vec![tree_root("fifo-root")], generous_limits()));
    });
    let fifo_error = receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("FIFO capture must not block waiting for a writer")
        .expect_err("FIFO must fail closed");
    fifo_worker
        .join()
        .expect("FIFO capture worker must not panic");
    assert_eq!(fifo_error.stage(), RepositoryCaptureStage::InspectFile);
    assert_eq!(
        fifo_error.kind(),
        RepositoryCaptureErrorKind::UnsupportedObject(RepositoryObjectKind::Fifo)
    );
    assert_eq!(
        fifo_error.path().map(RepositoryPath::as_str),
        Some("fifo-root/input.fifo")
    );

    fs::create_dir(fixture.path().join("socket-root")).expect("create socket tree root");
    let socket_path = fixture.path().join("socket-root/input.socket");
    // A u32 PID and a u128 hex nonce bound this ASCII path to at most 87 bytes.
    assert!(
        socket_path.as_os_str().as_bytes().len() < 104,
        "Unix socket fixture path must fit Darwin's sun_path"
    );
    let _listener = UnixListener::bind(&socket_path).expect("bind Unix socket fixture");
    let repository =
        RepositoryRoot::open_ambient(fixture.path()).expect("open special-file repository root");
    let socket_error = repository
        .capture(vec![tree_root("socket-root")], generous_limits())
        .expect_err("Unix socket must fail closed");
    assert!(
        matches!(
            (socket_error.stage(), socket_error.kind()),
            (
                RepositoryCaptureStage::OpenFile,
                RepositoryCaptureErrorKind::Backend | RepositoryCaptureErrorKind::Unsupported
            ) | (
                RepositoryCaptureStage::InspectFile,
                RepositoryCaptureErrorKind::UnsupportedObject(RepositoryObjectKind::Socket)
            )
        ),
        "unexpected Unix socket error: {socket_error:?}"
    );
    assert_eq!(
        socket_error.path().map(RepositoryPath::as_str),
        Some("socket-root/input.socket")
    );

    let device_repository = RepositoryRoot::open_ambient("/dev")
        .expect("Unix candidate runner must provide a /dev directory capability");
    let device_error = device_repository
        .capture(vec![file_root("null")], generous_limits())
        .expect_err("character device must fail closed");
    assert_eq!(device_error.stage(), RepositoryCaptureStage::InspectFile);
    assert_eq!(
        device_error.kind(),
        RepositoryCaptureErrorKind::ExpectedObject {
            expected: RepositoryObjectKind::File,
            actual: RepositoryObjectKind::CharacterDevice,
        }
    );
    assert_eq!(
        device_error.path().map(RepositoryPath::as_str),
        Some("null")
    );
}

#[cfg(windows)]
#[test]
fn windows_rejects_case_alias_root_spelling() {
    let upper = RepositoryPath::new("Source/Input.rs").expect("portable upper spelling");
    let lower = RepositoryPath::new("source/input.rs").expect("portable lower spelling");
    assert!(super::path::paths_portably_equal(&upper, &lower));
    assert_ne!(upper, lower);
}

#[cfg(windows)]
#[test]
fn windows_repository_capture_fails_closed_before_traversal() {
    let fixture = TempRoot::new("windows-identity");
    let repository = RepositoryRoot::open_ambient(fixture.path()).expect("open repository root");
    let error = repository
        .capture(vec![file_root("missing.bin")], generous_limits())
        .expect_err("Windows strict repository identity is not qualified");
    assert_eq!(
        error.kind(),
        RepositoryCaptureErrorKind::UnsupportedIdentityBackend
    );
    assert_eq!(error.stage(), RepositoryCaptureStage::ValidateRoots);
    assert!(error.path().is_none());
    assert!(std::error::Error::source(&error).is_none());
}

#[cfg(unix)]
fn assert_limit(
    result: Result<RepositoryCapture, RepositoryCaptureError>,
    expected: RepositoryLimit,
) {
    let error = result.expect_err("one-over resource use must fail");
    assert!(
        matches!(
            error.kind(),
            RepositoryCaptureErrorKind::LimitExceeded { limit, .. } if limit == expected
        ),
        "unexpected limit error: {error:?}"
    );
}
