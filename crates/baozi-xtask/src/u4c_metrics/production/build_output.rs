//! Strict Cargo build-script transcript and `OUT_DIR` closure capture.

#![cfg_attr(
    not(any(feature = "repository-input-candidate", test)),
    allow(
        dead_code,
        reason = "build-output admission is unreachable without the repository candidate feature"
    )
)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

const SHA256_HEX_LENGTH: usize = 64;
const REPOSITORY_PATH_HARD_MAX_BYTES: u64 = 4_096;
const REPOSITORY_TOKEN: &str = "${REPOSITORY_ROOT}";
const TARGET_TOKEN: &str = "${TARGET_DIR}";
const OUT_DIR_TOKEN: &str = "${OUT_DIR}";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BuildOutputSpecification {
    limits: BuildOutputLimitsSpecification,
    build_script_records: u64,
    out_directories: u64,
    files: u64,
    directories: u64,
    total_bytes: u64,
    transcript_sha256: String,
    tree_sha256: String,
}

impl BuildOutputSpecification {
    pub(super) fn validate_static(&self) -> Result<(), String> {
        self.limits.validate_static()?;
        for (name, value) in [
            ("build_script_records", self.build_script_records),
            ("out_directories", self.out_directories),
            ("files", self.files),
            ("directories", self.directories),
            ("total_bytes", self.total_bytes),
        ] {
            if value == 0 {
                return Err(format!(
                    "U4c production build_outputs.{name} must be positive"
                ));
            }
        }
        for (name, value) in [
            ("transcript_sha256", self.transcript_sha256.as_str()),
            ("tree_sha256", self.tree_sha256.as_str()),
        ] {
            if !is_lowercase_sha256(value) {
                return Err(format!(
                    "U4c production build_outputs.{name} must be a lowercase SHA-256"
                ));
            }
        }
        if self.out_directories > self.directories {
            return Err(
                "U4c production build_outputs.directories must include every OUT_DIR root"
                    .to_owned(),
            );
        }
        if self.out_directories > self.limits.max_roots {
            return Err(
                "U4c production build output limits.max_roots does not admit every OUT_DIR root"
                    .to_owned(),
            );
        }
        if self.files > self.limits.max_files {
            return Err(
                "U4c production build output limits.max_files is below the declared file count"
                    .to_owned(),
            );
        }
        if self.directories > self.limits.max_directories {
            return Err(
                "U4c production build output limits.max_directories is below the declared directory count"
                    .to_owned(),
            );
        }
        if self.total_bytes > self.limits.max_total_bytes {
            return Err(
                "U4c production build output limits.max_total_bytes is below the declared byte count"
                    .to_owned(),
            );
        }
        let entries = self
            .files
            .checked_add(self.directories)
            .ok_or_else(|| "U4c production build output entry accounting overflowed".to_owned())?;
        if entries > self.limits.max_entries {
            return Err(
                "U4c production build output limits.max_entries is below the declared closure"
                    .to_owned(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildOutputLimitsSpecification {
    max_roots: u64,
    max_entries: u64,
    max_files: u64,
    max_directories: u64,
    max_depth: u64,
    max_path_bytes: u64,
    max_file_bytes: u64,
    max_total_bytes: u64,
    max_work: u64,
}

impl BuildOutputLimitsSpecification {
    fn validate_static(&self) -> Result<(), String> {
        for (name, value) in [
            ("max_roots", self.max_roots),
            ("max_entries", self.max_entries),
            ("max_files", self.max_files),
            ("max_directories", self.max_directories),
            ("max_depth", self.max_depth),
            ("max_path_bytes", self.max_path_bytes),
            ("max_file_bytes", self.max_file_bytes),
            ("max_total_bytes", self.max_total_bytes),
            ("max_work", self.max_work),
        ] {
            if value == 0 {
                return Err(format!(
                    "U4c production build output limits.{name} must be nonzero"
                ));
            }
        }
        if self.max_path_bytes > REPOSITORY_PATH_HARD_MAX_BYTES {
            return Err(format!(
                "U4c production build output limits.max_path_bytes {} exceeds the hard maximum {REPOSITORY_PATH_HARD_MAX_BYTES}",
                self.max_path_bytes
            ));
        }
        self.max_files
            .checked_add(self.max_directories)
            .ok_or_else(|| "U4c production build output limit accounting overflowed".to_owned())?;
        for (name, value) in [
            ("max_roots", self.max_roots),
            ("max_entries", self.max_entries),
            ("max_files", self.max_files),
            ("max_directories", self.max_directories),
            ("max_depth", self.max_depth),
            ("max_file_bytes", self.max_file_bytes),
            ("max_total_bytes", self.max_total_bytes),
        ] {
            usize::try_from(value).map_err(|_| {
                format!("U4c production build output limits.{name} exceeds platform capacity")
            })?;
        }
        Ok(())
    }

    #[cfg(feature = "repository-input-candidate")]
    fn repository_limits(self) -> Result<baozi_io::RepositoryLimits, String> {
        self.validate_static()?;
        baozi_io::RepositoryLimitSpec {
            max_roots: self.max_roots,
            max_entries: self.max_entries,
            max_files: self.max_files,
            max_directories: self.max_directories,
            max_depth: self.max_depth,
            max_path_bytes: self.max_path_bytes,
            max_file_bytes: self.max_file_bytes,
            max_total_bytes: self.max_total_bytes,
            max_work: self.max_work,
        }
        .validate()
        .map_err(|error| format!("invalid U4c build output capture limits: {error}"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(super) struct BuildPackageIdentity {
    name: String,
    version: String,
    source: Option<String>,
    repository_manifest: Option<String>,
}

impl BuildPackageIdentity {
    pub(super) fn new(
        name: String,
        version: String,
        source: Option<String>,
        repository_manifest: Option<String>,
    ) -> Self {
        Self {
            name,
            version,
            source,
            repository_manifest,
        }
    }

    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn version(&self) -> &str {
        &self.version
    }

    pub(super) fn repository_manifest(&self) -> Option<&str> {
        self.repository_manifest.as_deref()
    }

    pub(super) fn package_reference(&self) -> BuildPackageReference {
        BuildPackageReference::new(self.name.clone(), self.version.clone())
    }

    fn validate(&self) -> Result<(), String> {
        validate_package_component(&self.name, "package name")?;
        validate_package_component(&self.version, "package version")?;
        match (&self.source, &self.repository_manifest) {
            (Some(source), None) if !source.is_empty() => Ok(()),
            (None, Some(manifest)) => {
                validate_repository_relative_path(manifest, "local package manifest")
            }
            (Some(_), Some(_)) => Err(format!(
                "build package {}@{} cannot have both source and repository_manifest",
                self.name, self.version
            )),
            (Some(_), None) => Err(format!(
                "build package {}@{} has an empty source",
                self.name, self.version
            )),
            (None, None) => Err(format!(
                "build package {}@{} must have source or repository_manifest",
                self.name, self.version
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct BuildPackageReference {
    name: String,
    version: String,
}

impl BuildPackageReference {
    pub(super) fn new(name: String, version: String) -> Self {
        Self { name, version }
    }

    fn validate(&self) -> Result<(), String> {
        validate_package_component(&self.name, "build-script package name")?;
        validate_package_component(&self.version, "build-script package version")
    }
}

#[derive(Debug)]
pub(super) struct PreparedCargoArtifact {
    executable: PathBuf,
    target_dir: PathBuf,
    build_script_records: BTreeMap<String, Vec<u8>>,
}

impl PreparedCargoArtifact {
    pub(super) fn executable(&self) -> &Path {
        &self.executable
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PreparedArtifactExpectation<'a> {
    root_package_name: &'a str,
    root_repository_manifest: &'a str,
    cargo_target_name: &'a str,
    test_profile: bool,
}

impl<'a> PreparedArtifactExpectation<'a> {
    pub(super) fn new(
        root_package_name: &'a str,
        root_repository_manifest: &'a str,
        cargo_target_name: &'a str,
        test_profile: bool,
    ) -> Self {
        Self {
            root_package_name,
            root_repository_manifest,
            cargo_target_name,
            test_profile,
        }
    }

    fn validate(self) -> Result<(), String> {
        validate_package_component(self.root_package_name, "expected root package name")?;
        validate_repository_relative_path(
            self.root_repository_manifest,
            "expected root package manifest",
        )?;
        validate_package_component(self.cargo_target_name, "expected Cargo target name")
    }
}

#[derive(Debug)]
struct MergedBuildOutputs {
    target_dir: PathBuf,
    build_script_records: BTreeMap<String, Vec<u8>>,
}

impl MergedBuildOutputs {
    fn transcript_sha256(&self) -> String {
        transcript_sha256(self.build_script_records.values().map(Vec::as_slice))
    }
}

pub(super) fn parse_prepared_artifact(
    stdout: &[u8],
    repository_root: &Path,
    target_dir: &Path,
    package_by_id: &BTreeMap<String, BuildPackageIdentity>,
    expected_build_scripts: &BTreeSet<BuildPackageReference>,
    expectation: PreparedArtifactExpectation<'_>,
) -> Result<PreparedCargoArtifact, String> {
    validate_absolute_path(repository_root, "repository root")?;
    validate_absolute_path(target_dir, "Cargo target directory")?;
    expectation.validate()?;
    for (package_id, identity) in package_by_id {
        if package_id.is_empty() {
            return Err("Cargo metadata contained an empty package id".to_owned());
        }
        identity.validate()?;
    }
    for package in expected_build_scripts {
        package.validate()?;
    }
    if stdout.is_empty() {
        return Err("Cargo emitted no JSON messages while preparing an artifact".to_owned());
    }

    let mut executable = None;
    let mut build_script_records = BTreeMap::new();
    let mut build_finished = false;
    let mut message_count = 0_u64;

    for (line_index, raw_line) in stdout.split_inclusive(|byte| *byte == b'\n').enumerate() {
        let line_number = line_index + 1;
        let line = raw_line.strip_suffix(b"\n").unwrap_or(raw_line);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            return Err(format!("Cargo JSON message line {line_number} was empty"));
        }
        if build_finished {
            return Err(format!(
                "Cargo emitted a message after build-finished on line {line_number}"
            ));
        }
        let mut message = serde_json::from_slice::<Value>(line).map_err(|error| {
            format!("Cargo emitted invalid JSON on line {line_number}: {error}")
        })?;
        let object = message
            .as_object()
            .ok_or_else(|| format!("Cargo JSON message line {line_number} was not an object"))?;
        let reason = object
            .get("reason")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                format!("Cargo JSON message line {line_number} omitted string field \"reason\"")
            })?
            .to_owned();
        message_count = message_count.checked_add(1).ok_or_else(|| {
            "Cargo JSON message count overflowed while preparing an artifact".to_owned()
        })?;

        match reason.as_str() {
            "compiler-message" => {
                let package_id =
                    required_json_string(&message, "package_id", "compiler-message", line_number)?;
                if !package_by_id.contains_key(package_id) {
                    return Err(format!(
                        "Cargo compiler-message line {line_number} referenced unknown package id {package_id:?}"
                    ));
                }
            }
            "compiler-artifact" => {
                inspect_compiler_artifact(
                    &message,
                    line_number,
                    target_dir,
                    package_by_id,
                    expectation,
                    &mut executable,
                )?;
            }
            "build-script-executed" => {
                let package_id = required_json_string(
                    &message,
                    "package_id",
                    "build-script-executed",
                    line_number,
                )?;
                let identity = package_by_id.get(package_id).ok_or_else(|| {
                    format!(
                        "Cargo build-script-executed line {line_number} referenced unknown package id {package_id:?}"
                    )
                })?;
                let package_reference = identity.package_reference();
                if !expected_build_scripts.contains(&package_reference) {
                    return Err(format!(
                        "Cargo executed undeclared build script {}@{} on line {line_number}",
                        identity.name(),
                        identity.version()
                    ));
                }
                let out_dir = required_json_string(
                    &message,
                    "out_dir",
                    "build-script-executed",
                    line_number,
                )?;
                let out_dir_path = PathBuf::from(out_dir);
                let relative_out_dir =
                    strict_relative_path(&out_dir_path, target_dir, "Cargo build-script OUT_DIR")?;
                canonicalize_build_script_message(
                    &mut message,
                    identity,
                    repository_root,
                    target_dir,
                    &out_dir_path,
                )?;
                let transcript = serde_json::to_vec(&message).map_err(|error| {
                    format!(
                        "failed to serialize canonical build-script transcript on line {line_number}: {error}"
                    )
                })?;
                insert_build_script_record(
                    &mut build_script_records,
                    relative_out_dir,
                    transcript,
                    "Cargo output",
                )?;
            }
            "build-finished" => {
                let success = message
                    .get("success")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| {
                        format!(
                            "Cargo build-finished line {line_number} omitted boolean field \"success\""
                        )
                    })?;
                if !success {
                    return Err(format!(
                        "Cargo build-finished line {line_number} did not report success"
                    ));
                }
                build_finished = true;
            }
            _ => {
                return Err(format!(
                    "Cargo emitted unsupported JSON reason {reason:?} on line {line_number}"
                ));
            }
        }
    }

    if message_count == 0 {
        return Err("Cargo emitted no JSON messages while preparing an artifact".to_owned());
    }
    if !build_finished {
        return Err(
            "Cargo artifact preparation must emit exactly one successful build-finished message"
                .to_owned(),
        );
    }
    let executable = executable.ok_or_else(|| {
        format!(
            "Cargo did not emit exactly one executable artifact for root package {:?} target {:?}",
            expectation.root_package_name, expectation.cargo_target_name
        )
    })?;

    Ok(PreparedCargoArtifact {
        executable,
        target_dir: target_dir.to_path_buf(),
        build_script_records,
    })
}

fn merge_prepared_artifacts(
    prepared_artifacts: &[&PreparedCargoArtifact],
) -> Result<MergedBuildOutputs, String> {
    let first = prepared_artifacts.first().ok_or_else(|| {
        "U4c build output capture requires at least one prepared Cargo artifact".to_owned()
    })?;
    let target_dir = first.target_dir.clone();
    let mut build_script_records = BTreeMap::new();
    for prepared in prepared_artifacts {
        if prepared.target_dir != target_dir {
            return Err(format!(
                "prepared Cargo artifacts used different target directories: {} and {}",
                target_dir.display(),
                prepared.target_dir.display()
            ));
        }
        for (out_dir, transcript) in &prepared.build_script_records {
            insert_build_script_record(
                &mut build_script_records,
                out_dir.clone(),
                transcript.clone(),
                "prepared Cargo artifacts",
            )?;
        }
    }
    Ok(MergedBuildOutputs {
        target_dir,
        build_script_records,
    })
}

#[cfg(feature = "repository-input-candidate")]
pub(super) struct BuildOutputAuthority {
    target_dir: PathBuf,
    repository: baozi_io::RepositoryRoot,
}

#[cfg(feature = "repository-input-candidate")]
impl BuildOutputAuthority {
    pub(super) fn open(target_dir: &Path) -> Result<Self, String> {
        validate_absolute_path(target_dir, "Cargo target directory")?;
        let repository = baozi_io::RepositoryRoot::open_ambient(target_dir).map_err(|error| {
            format!(
                "failed to open the U4c Cargo target capture authority {}: {error}",
                target_dir.display()
            )
        })?;
        Ok(Self {
            target_dir: target_dir.to_path_buf(),
            repository,
        })
    }

    pub(super) fn capture(
        &self,
        specification: &BuildOutputSpecification,
        prepared_artifacts: &[&PreparedCargoArtifact],
    ) -> Result<RetainedBuildOutputCapture, String> {
        specification.validate_static()?;
        let merged = merge_prepared_artifacts(prepared_artifacts)?;
        if merged.target_dir != self.target_dir {
            return Err(format!(
                "prepared Cargo artifacts target {} does not match build output authority {}",
                merged.target_dir.display(),
                self.target_dir.display()
            ));
        }

        let record_count = u64::try_from(merged.build_script_records.len())
            .map_err(|_| "U4c build-script record count exceeds u64".to_owned())?;

        let roots = merged
            .build_script_records
            .keys()
            .map(|path| {
                baozi_io::RepositoryPath::new(path)
                    .map(baozi_io::RepositoryRootSpec::Tree)
                    .map_err(|error| {
                        format!("invalid captured Cargo OUT_DIR path {path:?}: {error}")
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let limits = specification.limits.repository_limits()?;
        let capture = self
            .repository
            .capture(roots, limits)
            .map_err(|error| format!("failed to capture the U4c Cargo OUT_DIR closure: {error}"))?;
        let usage = capture.usage();

        let transcript_sha256 = merged.transcript_sha256();
        let tree_sha256 = build_output_tree_sha256(&merged.build_script_records, &capture)?;
        let failures = [
            require_count(
                "build-script records",
                specification.build_script_records,
                record_count,
            ),
            require_count(
                "declared OUT_DIR records",
                specification.out_directories,
                record_count,
            ),
            require_count(
                "captured OUT_DIR roots",
                specification.out_directories,
                usage.roots(),
            ),
            require_count("files", specification.files, usage.files()),
            require_count(
                "directories",
                specification.directories,
                usage.directories(),
            ),
            require_count(
                "total bytes",
                specification.total_bytes,
                usage.total_bytes(),
            ),
            verify_digest(
                "U4c build-script transcript",
                &specification.transcript_sha256,
                &transcript_sha256,
            ),
            verify_digest(
                "U4c build output tree",
                &specification.tree_sha256,
                &tree_sha256,
            ),
        ]
        .into_iter()
        .filter_map(Result::err)
        .collect::<Vec<_>>();
        if !failures.is_empty() {
            return Err(format!(
                "{}\nobserved build-output specification:\n\
                 build_script_records = {record_count}\n\
                 out_directories = {}\n\
                 files = {}\n\
                 directories = {}\n\
                 total_bytes = {}\n\
                 transcript_sha256 = {transcript_sha256:?}\n\
                 tree_sha256 = {tree_sha256:?}",
                failures.join("\n"),
                usage.roots(),
                usage.files(),
                usage.directories(),
                usage.total_bytes(),
            ));
        }

        Ok(RetainedBuildOutputCapture {
            capture,
            build_script_records: record_count,
            transcript_sha256,
            tree_sha256,
        })
    }

    pub(super) fn verify_unchanged(
        &self,
        retained: &RetainedBuildOutputCapture,
    ) -> Result<(), String> {
        self.repository
            .verify_unchanged(&retained.capture)
            .map_err(|error| {
                format!("U4c Cargo build outputs changed during production measurement: {error}")
            })
    }
}

#[cfg(feature = "repository-input-candidate")]
pub(super) struct RetainedBuildOutputCapture {
    capture: baozi_io::RepositoryCapture,
    build_script_records: u64,
    transcript_sha256: String,
    tree_sha256: String,
}

#[cfg(feature = "repository-input-candidate")]
impl RetainedBuildOutputCapture {
    pub(super) fn build_script_records(&self) -> u64 {
        self.build_script_records
    }

    pub(super) fn out_directories(&self) -> u64 {
        self.capture.usage().roots()
    }

    pub(super) fn files(&self) -> u64 {
        self.capture.usage().files()
    }

    pub(super) fn directories(&self) -> u64 {
        self.capture.usage().directories()
    }

    pub(super) fn total_bytes(&self) -> u64 {
        self.capture.usage().total_bytes()
    }

    pub(super) fn transcript_sha256(&self) -> &str {
        &self.transcript_sha256
    }

    pub(super) fn tree_sha256(&self) -> &str {
        &self.tree_sha256
    }
}

fn inspect_compiler_artifact(
    message: &Value,
    line_number: usize,
    target_dir: &Path,
    package_by_id: &BTreeMap<String, BuildPackageIdentity>,
    expectation: PreparedArtifactExpectation<'_>,
    executable: &mut Option<PathBuf>,
) -> Result<(), String> {
    let package_id = required_json_string(message, "package_id", "compiler-artifact", line_number)?;
    let identity = package_by_id.get(package_id).ok_or_else(|| {
        format!(
            "Cargo compiler-artifact line {line_number} referenced unknown package id {package_id:?}"
        )
    })?;
    let profile = message
        .get("profile")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            format!("Cargo compiler-artifact line {line_number} omitted object field \"profile\"")
        })?;
    let is_test = profile
        .get("test")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            format!(
                "Cargo compiler-artifact line {line_number} omitted boolean field \"profile.test\""
            )
        })?;
    let target = message
        .get("target")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            format!("Cargo compiler-artifact line {line_number} omitted object field \"target\"")
        })?;
    let target_name = target.get("name").and_then(Value::as_str).ok_or_else(|| {
        format!("Cargo compiler-artifact line {line_number} omitted string field \"target.name\"")
    })?;
    if is_test != expectation.test_profile
        || identity.name() != expectation.root_package_name
        || identity.repository_manifest() != Some(expectation.root_repository_manifest)
        || target_name != expectation.cargo_target_name
    {
        return Ok(());
    }

    let executable_path = message
        .get("executable")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            format!(
                "Cargo compiler-artifact line {line_number} for the expected test omitted its executable"
            )
        })?;
    let executable_path = Path::new(executable_path);
    strict_relative_path(
        executable_path,
        target_dir,
        "prepared Cargo executable artifact",
    )?;
    if let Some(previous) = executable {
        return Err(format!(
            "Cargo emitted multiple executable artifacts for root package {:?} target {:?}: {} and {}",
            expectation.root_package_name,
            expectation.cargo_target_name,
            previous.display(),
            executable_path.display()
        ));
    }
    *executable = Some(executable_path.to_path_buf());
    Ok(())
}

fn required_json_string<'a>(
    message: &'a Value,
    field: &str,
    reason: &str,
    line_number: usize,
) -> Result<&'a str, String> {
    message
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Cargo {reason} line {line_number} omitted string field {field:?}"))
}

fn canonicalize_build_script_message(
    message: &mut Value,
    identity: &BuildPackageIdentity,
    repository_root: &Path,
    target_dir: &Path,
    out_dir: &Path,
) -> Result<(), String> {
    let object = message
        .as_object_mut()
        .ok_or_else(|| "Cargo build-script-executed message was not a JSON object".to_owned())?;
    let stable_identity = serde_json::to_value(identity)
        .map_err(|error| format!("failed to serialize stable Cargo package identity: {error}"))?;
    object.insert("package_id".to_owned(), stable_identity);

    let normalizer = PathNormalizer::new(repository_root, target_dir, out_dir)?;
    normalize_json_strings(message, &normalizer)
}

fn normalize_json_strings(value: &mut Value, normalizer: &PathNormalizer) -> Result<(), String> {
    match value {
        Value::String(text) => {
            if [REPOSITORY_TOKEN, TARGET_TOKEN, OUT_DIR_TOKEN]
                .iter()
                .any(|token| text.contains(token))
            {
                return Err(format!(
                    "Cargo build-script transcript contains reserved path token in {text:?}"
                ));
            }
            *text = normalizer.normalize(text);
        }
        Value::Array(values) => {
            for value in values {
                normalize_json_strings(value, normalizer)?;
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                normalize_json_strings(value, normalizer)?;
            }
            values.sort_keys();
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    Ok(())
}

struct PathNormalizer {
    prefixes: Vec<(String, &'static str)>,
}

impl PathNormalizer {
    fn new(repository_root: &Path, target_dir: &Path, out_dir: &Path) -> Result<Self, String> {
        validate_absolute_path(repository_root, "repository root")?;
        validate_absolute_path(target_dir, "Cargo target directory")?;
        validate_absolute_path(out_dir, "Cargo build-script OUT_DIR")?;
        let mut prefixes = Vec::new();
        add_path_prefix(&mut prefixes, out_dir, OUT_DIR_TOKEN)?;
        add_path_prefix(&mut prefixes, target_dir, TARGET_TOKEN)?;
        add_path_prefix(&mut prefixes, repository_root, REPOSITORY_TOKEN)?;
        prefixes.sort_by(|left, right| {
            right
                .0
                .len()
                .cmp(&left.0.len())
                .then_with(|| left.0.cmp(&right.0))
        });
        Ok(Self { prefixes })
    }

    fn normalize(&self, text: &str) -> String {
        self.prefixes
            .iter()
            .fold(text.to_owned(), |normalized, (prefix, token)| {
                replace_bounded_path_prefix(&normalized, prefix, token)
            })
    }
}

fn add_path_prefix(
    prefixes: &mut Vec<(String, &'static str)>,
    path: &Path,
    token: &'static str,
) -> Result<(), String> {
    let native = path
        .to_str()
        .ok_or_else(|| format!("path {} is not valid UTF-8", path.display()))?;
    let native = native.trim_end_matches(['/', '\\']);
    if native.is_empty() {
        return Err("path normalization cannot bind a filesystem root".to_owned());
    }
    for spelling in [native.to_owned(), native.replace('\\', "/")] {
        if !prefixes
            .iter()
            .any(|(existing, existing_token)| existing == &spelling && *existing_token == token)
        {
            prefixes.push((spelling, token));
        }
    }
    Ok(())
}

fn replace_bounded_path_prefix(input: &str, prefix: &str, replacement: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(found) = input[cursor..].find(prefix) {
        let start = cursor + found;
        let end = start + prefix.len();
        let preceding = start
            .checked_sub(1)
            .and_then(|index| input.as_bytes().get(index).copied());
        let following = input.as_bytes().get(end).copied();
        if preceding.is_some_and(|byte| !is_path_prefix_boundary(byte))
            || following.is_some_and(|byte| !is_path_prefix_boundary(byte))
        {
            output.push_str(&input[cursor..end]);
            cursor = end;
            continue;
        }
        output.push_str(&input[cursor..start]);
        output.push_str(replacement);
        if following.is_some_and(|byte| matches!(byte, b'/' | b'\\')) {
            output.push('/');
            cursor = end + 1;
        } else {
            cursor = end;
        }
    }
    output.push_str(&input[cursor..]);
    output
}

fn is_path_prefix_boundary(byte: u8) -> bool {
    matches!(byte, b'/' | b'\\' | b'#' | b'?' | b';' | b',' | b':' | b'=')
        || byte.is_ascii_whitespace()
        || matches!(byte, b')' | b']' | b'}' | b'\'' | b'"')
}

fn insert_build_script_record(
    records: &mut BTreeMap<String, Vec<u8>>,
    out_dir: String,
    transcript: Vec<u8>,
    source: &str,
) -> Result<(), String> {
    match records.entry(out_dir) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(transcript);
            Ok(())
        }
        std::collections::btree_map::Entry::Occupied(entry)
            if entry.get().as_slice() == transcript.as_slice() =>
        {
            Ok(())
        }
        std::collections::btree_map::Entry::Occupied(entry) => Err(format!(
            "{source} replayed OUT_DIR {:?} with a conflicting build-script transcript",
            entry.key()
        )),
    }
}

fn validate_package_component(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty() || !value.is_ascii() || value.chars().any(char::is_whitespace) {
        return Err(format!(
            "{label} {value:?} must be nonempty ASCII without whitespace"
        ));
    }
    Ok(())
}

fn validate_repository_relative_path(path: &str, label: &str) -> Result<(), String> {
    if path.is_empty()
        || !path.is_ascii()
        || path.contains('\\')
        || path.starts_with('/')
        || path.ends_with('/')
    {
        return Err(format!(
            "{label} path {path:?} must be a nonempty ASCII forward-slash relative path"
        ));
    }
    if path
        .split('/')
        .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return Err(format!(
            "{label} path {path:?} must contain only normal components"
        ));
    }
    if Path::new(path)
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "{label} path {path:?} must not contain a prefix, root, or dot component"
        ));
    }
    Ok(())
}

fn validate_absolute_path(path: &Path, label: &str) -> Result<(), String> {
    let text = path
        .to_str()
        .ok_or_else(|| format!("{label} path {} is not valid UTF-8", path.display()))?;
    if !text.is_ascii() {
        return Err(format!("{label} path {text:?} must be ASCII"));
    }
    if !path.is_absolute() {
        return Err(format!("{label} path {text:?} must be absolute"));
    }
    if text
        .split(['/', '\\'])
        .any(|component| matches!(component, "." | ".."))
    {
        return Err(format!(
            "{label} path {text:?} must not contain dot components"
        ));
    }

    let mut saw_root = false;
    let mut saw_normal = false;
    for component in path.components() {
        match component {
            Component::Prefix(_) if !saw_root && !saw_normal => {}
            Component::RootDir if !saw_root && !saw_normal => saw_root = true,
            Component::Normal(component) if saw_root => {
                let component = component
                    .to_str()
                    .ok_or_else(|| format!("{label} path {text:?} contains invalid UTF-8"))?;
                if !component.is_ascii() || component.contains(['/', '\\']) {
                    return Err(format!(
                        "{label} path {text:?} contains a nonportable component"
                    ));
                }
                saw_normal = true;
            }
            _ => {
                return Err(format!(
                    "{label} path {text:?} must contain only an absolute root and normal components"
                ));
            }
        }
    }
    if !saw_root || !saw_normal {
        return Err(format!(
            "{label} path {text:?} must name a directory below the filesystem root"
        ));
    }
    Ok(())
}

fn strict_relative_path(path: &Path, root: &Path, label: &str) -> Result<String, String> {
    validate_absolute_path(path, label)?;
    let relative = path.strip_prefix(root).map_err(|_| {
        format!(
            "{label} {} is outside Cargo target directory {}",
            path.display(),
            root.display()
        )
    })?;
    let mut components = Vec::new();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(format!(
                "{label} {} contains an escaping or non-normal component",
                path.display()
            ));
        };
        let component = component
            .to_str()
            .ok_or_else(|| format!("{label} {} is not valid UTF-8", path.display()))?;
        if !component.is_ascii() || component.contains(['/', '\\']) {
            return Err(format!(
                "{label} {} contains a non-ASCII or nonportable component",
                path.display()
            ));
        }
        components.push(component);
    }
    if components.is_empty() {
        return Err(format!(
            "{label} {} must be strictly below Cargo target directory {}",
            path.display(),
            root.display()
        ));
    }
    let relative = components.join("/");
    validate_repository_relative_path(&relative, label)?;
    Ok(relative)
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == SHA256_HEX_LENGTH
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn transcript_sha256<'a>(transcripts: impl IntoIterator<Item = &'a [u8]>) -> String {
    let mut transcripts = transcripts.into_iter().collect::<Vec<_>>();
    transcripts.sort_unstable();
    let mut digest = Sha256::new();
    for transcript in transcripts {
        digest.update(transcript);
        digest.update(b"\n");
    }
    format!("{:x}", digest.finalize())
}

fn tree_content_sha256<'a>(
    directories: impl IntoIterator<Item = &'a str>,
    files: impl IntoIterator<Item = (&'a str, &'a [u8])>,
) -> String {
    let mut directories = directories.into_iter().collect::<Vec<_>>();
    directories.sort_unstable();
    let mut files = files
        .into_iter()
        .map(|(path, bytes)| (path, sha256_hex(bytes)))
        .collect::<Vec<_>>();
    files.sort_unstable_by_key(|(path, _)| *path);
    let mut digest = Sha256::new();
    for path in directories {
        digest.update(b"D");
        digest.update(path.as_bytes());
        digest.update(b"\n");
    }
    for (path, file_sha256) in files {
        digest.update(b"F");
        digest.update(path.as_bytes());
        digest.update([0]);
        digest.update(file_sha256.as_bytes());
        digest.update(b"\n");
    }
    format!("{:x}", digest.finalize())
}

#[cfg(feature = "repository-input-candidate")]
fn build_output_tree_sha256(
    build_script_records: &BTreeMap<String, Vec<u8>>,
    capture: &baozi_io::RepositoryCapture,
) -> Result<String, String> {
    let mut aggregate_records = Vec::with_capacity(build_script_records.len());
    for (out_dir, transcript) in build_script_records {
        let directories = capture
            .directory_paths()
            .filter_map(|path| relative_to_out_dir(path.as_str(), out_dir))
            .collect::<Vec<_>>();
        let files = capture
            .files()
            .iter()
            .filter_map(|file| {
                relative_to_out_dir(file.path().as_str(), out_dir).map(|path| (path, file.bytes()))
            })
            .collect::<Vec<_>>();
        if !directories.iter().any(|path| path.is_empty()) {
            return Err(format!(
                "captured U4c build output omitted declared OUT_DIR root {out_dir:?}"
            ));
        }
        let tree_sha256 = tree_content_sha256(directories, files);
        aggregate_records.push(format!("{}\0{tree_sha256}\n", sha256_hex(transcript)));
    }
    aggregate_records.sort_unstable();
    let mut digest = Sha256::new();
    for record in aggregate_records {
        digest.update(record.as_bytes());
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn relative_to_out_dir<'path>(path: &'path str, out_dir: &str) -> Option<&'path str> {
    if path == out_dir {
        return Some("");
    }
    path.strip_prefix(out_dir)?.strip_prefix('/')
}

fn verify_digest(label: &str, expected: &str, actual: &str) -> Result<(), String> {
    if expected == actual {
        Ok(())
    } else {
        Err(format!(
            "{label} drifted: expected {expected}, found {actual}"
        ))
    }
}

fn require_count(label: &str, expected: u64, actual: u64) -> Result<(), String> {
    if expected == actual {
        Ok(())
    } else {
        Err(format!(
            "U4c build output {label} drifted: expected {expected}, found {actual}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PACKAGE_ID: &str = "path+file:///u4c-repository/crates/fixture#1.2.3";

    fn repository_root() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\u4c-repository")
        } else {
            PathBuf::from("/u4c-repository")
        }
    }

    fn target_dir() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\u4c-target")
        } else {
            PathBuf::from("/u4c-target")
        }
    }

    fn target_path(relative: &str) -> PathBuf {
        relative
            .split('/')
            .fold(target_dir(), |path, component| path.join(component))
    }

    fn packages() -> BTreeMap<String, BuildPackageIdentity> {
        BTreeMap::from([(
            PACKAGE_ID.to_owned(),
            BuildPackageIdentity::new(
                "fixture".to_owned(),
                "1.2.3".to_owned(),
                None,
                Some("crates/fixture/Cargo.toml".to_owned()),
            ),
        )])
    }

    fn declared_build_scripts() -> BTreeSet<BuildPackageReference> {
        BTreeSet::from([BuildPackageReference::new(
            "fixture".to_owned(),
            "1.2.3".to_owned(),
        )])
    }

    fn json_lines(messages: impl IntoIterator<Item = Value>) -> Vec<u8> {
        let mut output = Vec::new();
        for message in messages {
            output.extend_from_slice(
                &serde_json::to_vec(&message).expect("test JSON must serialize"),
            );
            output.push(b'\n');
        }
        output
    }

    fn build_script_message(out_dir: &Path, marker: u64) -> Value {
        json!({
            "reason": "build-script-executed",
            "package_id": PACKAGE_ID,
            "linked_libs": [],
            "linked_paths": [],
            "cfgs": [],
            "env": [],
            "out_dir": out_dir,
            "future_field": {
                "marker": marker,
                "repository_path": repository_root().join("crates/fixture"),
            },
        })
    }

    fn artifact_message(executable: &Path) -> Value {
        artifact_message_with_profile(executable, true)
    }

    fn artifact_message_with_profile(executable: &Path, test_profile: bool) -> Value {
        json!({
            "reason": "compiler-artifact",
            "package_id": PACKAGE_ID,
            "target": { "name": "fixture" },
            "profile": { "test": test_profile },
            "executable": executable,
        })
    }

    fn parse(stdout: &[u8]) -> Result<PreparedCargoArtifact, String> {
        parse_prepared_artifact(
            stdout,
            &repository_root(),
            &target_dir(),
            &packages(),
            &declared_build_scripts(),
            PreparedArtifactExpectation::new(
                "fixture",
                "crates/fixture/Cargo.toml",
                "fixture",
                true,
            ),
        )
    }

    #[test]
    fn rejects_unknown_cargo_reason() {
        let error = parse(&json_lines([json!({ "reason": "future-reason" })]))
            .expect_err("unknown Cargo reasons must fail closed");
        assert!(error.contains("unsupported JSON reason"));
        assert!(error.contains("future-reason"));
    }

    #[test]
    fn rejects_compiler_message_for_unknown_package() {
        let executable = target_path("debug/deps/fixture-test");
        let stdout = json_lines([
            json!({
                "reason": "compiler-message",
                "package_id": "registry+https://example.invalid/index#unknown@9.9.9",
            }),
            artifact_message(&executable),
            json!({ "reason": "build-finished", "success": true }),
        ]);
        let error = parse(&stdout).expect_err("compiler messages must use admitted package ids");

        assert!(error.contains("compiler-message"));
        assert!(error.contains("unknown package id"));
    }

    #[test]
    fn rejects_undeclared_build_script() {
        let stdout = json_lines([build_script_message(
            &target_path("debug/build/fixture/out"),
            1,
        )]);
        let error = parse_prepared_artifact(
            &stdout,
            &repository_root(),
            &target_dir(),
            &packages(),
            &BTreeSet::new(),
            PreparedArtifactExpectation::new(
                "fixture",
                "crates/fixture/Cargo.toml",
                "fixture",
                true,
            ),
        )
        .expect_err("an undeclared build script must fail closed");
        assert!(error.contains("undeclared build script"));
        assert!(error.contains("fixture@1.2.3"));
    }

    #[test]
    fn rejects_conflicting_replay_for_one_out_dir() {
        let out_dir = target_path("debug/build/fixture/out");
        let stdout = json_lines([
            build_script_message(&out_dir, 1),
            build_script_message(&out_dir, 2),
        ]);
        let error = parse(&stdout).expect_err("one OUT_DIR cannot have two transcripts");
        assert!(error.contains("conflicting build-script transcript"));
        assert!(error.contains("debug/build/fixture/out"));
    }

    #[test]
    fn rejects_out_dir_outside_target_directory() {
        let outside = if cfg!(windows) {
            PathBuf::from(r"C:\outside\build\fixture\out")
        } else {
            PathBuf::from("/outside/build/fixture/out")
        };
        let error = parse(&json_lines([build_script_message(&outside, 1)]))
            .expect_err("OUT_DIR outside target must fail closed");
        assert!(error.contains("outside Cargo target directory"));
    }

    #[test]
    fn accepts_identical_out_dir_replay_and_selects_unique_test() {
        let out_dir = target_path("debug/build/fixture/out");
        let executable = target_path("debug/deps/fixture-test");
        let message = build_script_message(&out_dir, 1);
        let stdout = json_lines([
            message.clone(),
            message,
            artifact_message(&executable),
            json!({ "reason": "build-finished", "success": true }),
        ]);
        let prepared = parse(&stdout).expect("valid Cargo output must parse");
        assert_eq!(prepared.executable(), executable);
        assert_eq!(prepared.build_script_records.len(), 1);
    }

    #[test]
    fn accepts_the_expected_non_test_binary_artifact() {
        let executable = target_path("debug/fixture");
        let stdout = json_lines([
            artifact_message_with_profile(&executable, false),
            json!({ "reason": "build-finished", "success": true }),
        ]);
        let prepared = parse_prepared_artifact(
            &stdout,
            &repository_root(),
            &target_dir(),
            &packages(),
            &declared_build_scripts(),
            PreparedArtifactExpectation::new(
                "fixture",
                "crates/fixture/Cargo.toml",
                "fixture",
                false,
            ),
        )
        .expect("the rebuilt production binary must be selectable");

        assert_eq!(prepared.executable(), executable);
    }

    #[test]
    fn does_not_select_a_same_name_non_root_test_artifact() {
        let executable = target_path("debug/deps/fixture-test");
        let stdout = json_lines([
            artifact_message(&executable),
            json!({ "reason": "build-finished", "success": true }),
        ]);
        let registry_identity = BuildPackageIdentity::new(
            "fixture".to_owned(),
            "1.2.3".to_owned(),
            Some("registry+https://example.invalid/index".to_owned()),
            None,
        );
        let error = parse_prepared_artifact(
            &stdout,
            &repository_root(),
            &target_dir(),
            &BTreeMap::from([(PACKAGE_ID.to_owned(), registry_identity)]),
            &declared_build_scripts(),
            PreparedArtifactExpectation::new(
                "fixture",
                "crates/fixture/Cargo.toml",
                "fixture",
                true,
            ),
        )
        .expect_err("a same-name dependency artifact must not become the root test");

        assert!(error.contains("did not emit exactly one executable artifact"));
    }

    #[test]
    fn canonical_transcript_retains_unknown_fields_in_digest() {
        let out_dir = target_path("debug/build/fixture/out");
        let mut first = build_script_message(&out_dir, 1);
        let mut second = build_script_message(&out_dir, 2);
        let identity = packages()
            .remove(PACKAGE_ID)
            .expect("fixture package must exist");
        canonicalize_build_script_message(
            &mut first,
            &identity,
            &repository_root(),
            &target_dir(),
            &out_dir,
        )
        .expect("first transcript must canonicalize");
        canonicalize_build_script_message(
            &mut second,
            &identity,
            &repository_root(),
            &target_dir(),
            &out_dir,
        )
        .expect("second transcript must canonicalize");
        let first = serde_json::to_vec(&first).expect("first JSON must serialize");
        let second = serde_json::to_vec(&second).expect("second JSON must serialize");
        assert_ne!(
            transcript_sha256([first.as_slice()]),
            transcript_sha256([second.as_slice()])
        );
        assert!(
            String::from_utf8(first)
                .expect("canonical JSON is UTF-8")
                .contains(REPOSITORY_TOKEN)
        );
    }

    #[test]
    fn path_normalization_requires_both_component_boundaries() {
        let normalizer = PathNormalizer::new(
            &repository_root(),
            &target_dir(),
            &target_path("debug/build/fixture/out"),
        )
        .expect("fixture paths must define a normalizer");
        let repository_root = repository_root();
        let repository = repository_root
            .to_str()
            .expect("fixture repository path is UTF-8");
        let embedded = format!("/outside{repository}/crates/fixture");

        assert_eq!(normalizer.normalize(&embedded), embedded);
        assert_eq!(
            normalizer.normalize(&repository_root.join("crates/fixture").to_string_lossy()),
            format!("{REPOSITORY_TOKEN}/crates/fixture")
        );
    }

    #[test]
    fn rejects_authored_reserved_path_tokens_before_normalization() {
        let out_dir = target_path("debug/build/fixture/out");
        let mut message = build_script_message(&out_dir, 1);
        message["env"] = json!([["COLLISION", format!("{OUT_DIR_TOKEN}/generated")]]);
        let identity = packages()
            .remove(PACKAGE_ID)
            .expect("fixture package must exist");
        let error = canonicalize_build_script_message(
            &mut message,
            &identity,
            &repository_root(),
            &target_dir(),
            &out_dir,
        )
        .expect_err("authored path tokens must not collide with normalized paths");

        assert!(error.contains("reserved path token"));
        assert!(error.contains(OUT_DIR_TOKEN));
    }

    #[test]
    fn digest_helpers_are_sorted_and_verify_exactly() {
        let first = tree_content_sha256(
            ["", "b", "a"],
            [
                ("b/file", b"second".as_slice()),
                ("a/file", b"first".as_slice()),
            ],
        );
        let reordered = tree_content_sha256(
            ["a", "", "b"],
            [
                ("a/file", b"first".as_slice()),
                ("b/file", b"second".as_slice()),
            ],
        );
        assert_eq!(first, reordered);
        assert_ne!(
            first,
            tree_content_sha256(
                ["", "a", "b"],
                [
                    ("a/file", b"changed".as_slice()),
                    ("b/file", b"second".as_slice()),
                ],
            )
        );
        assert_ne!(
            first,
            tree_content_sha256(
                ["", "a", "renamed"],
                [
                    ("a/file", b"first".as_slice()),
                    ("b/file", b"second".as_slice()),
                ],
            )
        );

        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(sha256_hex(b"abc"), abc);
        assert!(verify_digest("fixture", abc, abc).is_ok());
        assert!(verify_digest("fixture", abc, &"0".repeat(64)).is_err());
    }

    #[test]
    fn specification_rejects_unknown_fields() {
        let value = json!({
            "limits": {
                "max_roots": 1,
                "max_entries": 1,
                "max_files": 1,
                "max_directories": 1,
                "max_depth": 1,
                "max_path_bytes": 1,
                "max_file_bytes": 1,
                "max_total_bytes": 1,
                "max_work": 1,
            },
            "build_script_records": 1,
            "out_directories": 1,
            "files": 1,
            "directories": 1,
            "total_bytes": 1,
            "transcript_sha256": "0".repeat(64),
            "tree_sha256": "0".repeat(64),
            "unexpected": true,
        });
        assert!(serde_json::from_value::<BuildOutputSpecification>(value).is_err());
    }
}
