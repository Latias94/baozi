use super::{MetricsV2, PROBE_PREFIX, PROBE_TARGET};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const SPECIFICATION_SCHEMA_VERSION: u64 = 2;
const LIFECYCLE_DIAGNOSTIC_SCOPE: &str = "lifecycle-diagnostic";
const COMPLEX_FIXTURE_ID: &str = "canonical-store-complex-v1";
const FIXTURE_SOURCE: &str = "crates/baozi-core/src/canonical_scene/store/tests.rs";
const ACCOUNT_BASELINE: &str = "fresh-factory-and-account";
const RELEASE_BOUNDARY: &str = "terminal-parts-drop";
const OBSERVED_FACTS: &[&str] = &["ledger", "physical-account", "handoff", "release"];
const UNOBSERVED_FACTS: &[&str] = &[
    "semantic-digest",
    "semantic-mutation-vectors",
    "copy-events",
    "decode-events",
    "lookup-events",
    "producer-events",
    "complete-measurement-closure",
    "handle-relative-input-closure",
    "target-identity",
    "profile-identity",
    "libstd-identity",
    "allocator-identity",
];
const TOOLCHAIN_PROBE_ARGUMENTS: &[&str] = &[
    "rustc",
    "-j",
    "1",
    "--locked",
    "-p",
    "baozi-core",
    "--lib",
    "--",
    "-vV",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LifecycleDiagnosticSpec {
    schema_version: u64,
    evidence_scope: String,
    probe: ProbeSpec,
    toolchain: ToolchainSpec,
    fixture: FixtureSpec,
    accounting: AccountingSpec,
    coverage: CoverageSpec,
    source_anchors: Vec<SourceAnchor>,
    expected_metrics: MetricsV2,
    #[serde(skip)]
    specification_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeSpec {
    id: String,
    test_target: String,
    report_prefix: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolchainSpec {
    rustc_release: String,
    host: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureSpec {
    id: String,
    source: String,
    schema_revision: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountingSpec {
    baseline: String,
    release_boundary: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CoverageSpec {
    qualifies_u4_exit: bool,
    observed: Vec<String>,
    unobserved: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceAnchor {
    path: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct EvidenceScopeEnvelope {
    evidence_scope: String,
}

/// Reads only the routing discriminator for a non-qualifying diagnostic command.
///
/// Production evidence must use handle-relative input discovery and must not reuse this
/// canonicalize-then-open path reader.
pub(super) fn read_evidence_scope(
    repository_root: &Path,
    specification_path: &Path,
) -> Result<String, String> {
    let repository_root = canonical_directory(repository_root, "repository root")?;
    let specification_path = if specification_path.is_absolute() {
        specification_path.to_path_buf()
    } else {
        repository_root.join(specification_path)
    };
    let specification_path =
        canonical_file_within(&repository_root, &specification_path, "U4c specification")?;
    let contents = fs::read_to_string(&specification_path).map_err(|error| {
        format!(
            "failed to read U4c specification {}: {error}",
            specification_path.display()
        )
    })?;
    parse_evidence_scope(&contents)
}

fn parse_evidence_scope(contents: &str) -> Result<String, String> {
    let envelope = toml::from_str::<EvidenceScopeEnvelope>(contents)
        .map_err(|error| format!("failed to read U4c evidence_scope: {error}"))?;
    if envelope.evidence_scope.is_empty() {
        return Err("U4c evidence_scope must not be empty".to_owned());
    }
    Ok(envelope.evidence_scope)
}

impl LifecycleDiagnosticSpec {
    pub(super) fn load(repository_root: &Path, specification_path: &Path) -> Result<Self, String> {
        let repository_root = canonical_directory(repository_root, "repository root")?;
        let specification_path = if specification_path.is_absolute() {
            specification_path.to_path_buf()
        } else {
            repository_root.join(specification_path)
        };
        let specification_path =
            canonical_file_within(&repository_root, &specification_path, "U4c specification")?;
        let contents = fs::read_to_string(&specification_path).map_err(|error| {
            format!(
                "failed to read U4c specification {}: {error}",
                specification_path.display()
            )
        })?;
        let mut specification = parse_specification(&contents)?;
        specification.specification_sha256 =
            normalized_sha256_hex(contents.as_bytes(), &specification_path)?;
        specification.verify_source_anchors(&repository_root)?;
        Ok(specification)
    }

    pub(super) fn verify_toolchain(&self, repository_root: &Path) -> Result<(), String> {
        let repository_root = canonical_directory(repository_root, "repository root")?;
        let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
        let output = Command::new(cargo)
            .args(TOOLCHAIN_PROBE_ARGUMENTS)
            .current_dir(repository_root)
            .output()
            .map_err(|error| {
                format!("failed to run the Cargo-selected rustc for U4c diagnostics: {error}")
            })?;
        if !output.status.success() {
            return Err(format!(
                "Cargo-selected rustc probe failed for U4c diagnostics with {}: {}",
                output.status,
                stderr_diagnostic(&output.stderr)
            ));
        }

        let stdout = std::str::from_utf8(&output.stdout).map_err(|error| {
            format!("Cargo-selected rustc probe emitted non-UTF-8 stdout: {error}")
        })?;
        let release = metadata_line(stdout, "release")?;
        let host = metadata_line(stdout, "host")?;
        if release != self.toolchain.rustc_release {
            return Err(format!(
                "U4c lifecycle diagnostic rustc release drifted: expected {}, found {release}",
                self.toolchain.rustc_release
            ));
        }
        if host != self.toolchain.host {
            return Err(format!(
                "U4c lifecycle diagnostic host drifted: expected {}, found {host}",
                self.toolchain.host
            ));
        }

        Ok(())
    }

    pub(super) fn verify_report(&self, report: &MetricsV2) -> Result<(), String> {
        if report != &self.expected_metrics {
            let expected = serde_json::to_string(&self.expected_metrics)
                .map_err(|error| format!("could not serialize expected U4c metrics: {error}"))?;
            let actual = serde_json::to_string(report)
                .map_err(|error| format!("could not serialize observed U4c metrics: {error}"))?;
            return Err(format!(
                "U4c lifecycle diagnostic metrics drifted from {}: expected {expected}, found {actual}",
                self.fixture.id
            ));
        }

        Ok(())
    }

    pub(super) fn evidence_scope(&self) -> &str {
        &self.evidence_scope
    }

    pub(super) fn fixture_id(&self) -> &str {
        &self.fixture.id
    }

    pub(super) fn specification_sha256(&self) -> &str {
        &self.specification_sha256
    }

    fn validate_static(&self) -> Result<(), String> {
        if self.schema_version != SPECIFICATION_SCHEMA_VERSION {
            return Err(format!(
                "unsupported U4c lifecycle diagnostic schema_version {}; expected {SPECIFICATION_SCHEMA_VERSION}",
                self.schema_version
            ));
        }
        if self.evidence_scope != LIFECYCLE_DIAGNOSTIC_SCOPE {
            return Err(format!(
                "unsupported U4c evidence_scope {:?}; only {LIFECYCLE_DIAGNOSTIC_SCOPE:?} is implemented and it cannot qualify U4 exit",
                self.evidence_scope
            ));
        }
        if self.probe.id != "canonical-store-u4c-metrics-v2" {
            return Err(format!(
                "unsupported U4c lifecycle diagnostic probe id {:?}",
                self.probe.id
            ));
        }
        if self.probe.test_target != PROBE_TARGET {
            return Err(format!(
                "U4c lifecycle diagnostic probe target drifted: expected {PROBE_TARGET:?}, found {:?}",
                self.probe.test_target
            ));
        }
        if self.probe.report_prefix != PROBE_PREFIX {
            return Err(format!(
                "U4c lifecycle diagnostic report prefix drifted: expected {PROBE_PREFIX:?}, found {:?}",
                self.probe.report_prefix
            ));
        }
        if self.fixture.id != COMPLEX_FIXTURE_ID || self.fixture.source != FIXTURE_SOURCE {
            return Err(format!(
                "unsupported U4c lifecycle diagnostic fixture {:?} at {:?}",
                self.fixture.id, self.fixture.source
            ));
        }
        if self.accounting.baseline != ACCOUNT_BASELINE
            || self.accounting.release_boundary != RELEASE_BOUNDARY
        {
            return Err(format!(
                "U4c lifecycle diagnostic accounting must use baseline {ACCOUNT_BASELINE:?} and release boundary {RELEASE_BOUNDARY:?}"
            ));
        }
        if self.coverage.qualifies_u4_exit {
            return Err(
                "a lifecycle diagnostic specification must set qualifies_u4_exit to false"
                    .to_owned(),
            );
        }
        validate_exact_list("observed coverage", &self.coverage.observed, OBSERVED_FACTS)?;
        validate_exact_list(
            "unobserved coverage",
            &self.coverage.unobserved,
            UNOBSERVED_FACTS,
        )?;
        if self.toolchain.rustc_release.is_empty() || self.toolchain.host.is_empty() {
            return Err("U4c lifecycle diagnostic toolchain fields must not be empty".to_owned());
        }
        if self.expected_metrics.fixture_schema_revision != self.fixture.schema_revision {
            return Err(format!(
                "U4c lifecycle diagnostic expected fixture schema revision {} does not match fixture declaration {}",
                self.expected_metrics.fixture_schema_revision, self.fixture.schema_revision
            ));
        }
        self.expected_metrics.validate()?;

        Ok(())
    }

    pub(super) fn verify_source_anchors(&self, repository_root: &Path) -> Result<(), String> {
        let repository_root = canonical_directory(repository_root, "repository root")?;
        self.verify_source_anchors_from_canonical_root(&repository_root)
    }

    fn verify_source_anchors_from_canonical_root(
        &self,
        repository_root: &Path,
    ) -> Result<(), String> {
        if self.source_anchors.is_empty() {
            return Err("U4c lifecycle diagnostic requires at least one source anchor".to_owned());
        }

        let mut paths = BTreeSet::new();
        let mut canonical_paths = BTreeSet::new();
        let mut previous_path = None;
        let mut fixture_source_is_anchored = false;
        for anchor in &self.source_anchors {
            let relative_path = validated_relative_path(&anchor.path, "U4c source anchor")?;
            if !is_lowercase_sha256(&anchor.sha256) {
                return Err(format!(
                    "U4c source anchor {} has an invalid lowercase SHA-256 digest",
                    anchor.path
                ));
            }
            if !paths.insert(anchor.path.as_str()) {
                return Err(format!("U4c source anchor {} is duplicated", anchor.path));
            }
            if let Some(previous_path) = previous_path
                && previous_path >= anchor.path.as_str()
            {
                return Err(
                    "U4c source anchors must be sorted by normalized repository-relative path"
                        .to_owned(),
                );
            }
            previous_path = Some(anchor.path.as_str());

            let source_path = canonical_file_within(
                repository_root,
                &repository_root.join(relative_path),
                "U4c source anchor",
            )?;
            if !canonical_paths.insert(source_path.clone()) {
                return Err(format!(
                    "U4c source anchor {} duplicates a canonical source path",
                    anchor.path
                ));
            }
            let contents = fs::read(&source_path).map_err(|error| {
                format!(
                    "failed to read U4c source anchor {}: {error}",
                    source_path.display()
                )
            })?;
            let actual_digest = normalized_sha256_hex(&contents, &source_path)?;
            if actual_digest != anchor.sha256 {
                return Err(format!(
                    "U4c source anchor {} drifted: expected {}, found {actual_digest}",
                    anchor.path, anchor.sha256
                ));
            }
            fixture_source_is_anchored |= anchor.path == self.fixture.source;
        }

        if !fixture_source_is_anchored {
            return Err(format!(
                "U4c lifecycle diagnostic fixture source {} must appear in source_anchors",
                self.fixture.source
            ));
        }

        Ok(())
    }
}

fn parse_specification(contents: &str) -> Result<LifecycleDiagnosticSpec, String> {
    let specification = toml::from_str::<LifecycleDiagnosticSpec>(contents).map_err(|error| {
        format!("failed to parse U4c lifecycle diagnostic specification: {error}")
    })?;
    specification.validate_static()?;
    Ok(specification)
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("failed to canonicalize {label} {}: {error}", path.display()))?;
    if !canonical.is_dir() {
        return Err(format!(
            "{label} {} is not a directory",
            canonical.display()
        ));
    }
    Ok(canonical)
}

fn canonical_file_within(
    repository_root: &Path,
    path: &Path,
    label: &str,
) -> Result<PathBuf, String> {
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("failed to canonicalize {label} {}: {error}", path.display()))?;
    if !canonical.starts_with(repository_root) {
        return Err(format!(
            "{label} {} must remain within repository root {}",
            canonical.display(),
            repository_root.display()
        ));
    }
    if !canonical.is_file() {
        return Err(format!("{label} {} is not a file", canonical.display()));
    }
    Ok(canonical)
}

fn validated_relative_path<'a>(path: &'a str, label: &str) -> Result<&'a Path, String> {
    if path.is_empty()
        || path.contains('\\')
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains("//")
    {
        return Err(format!(
            "{label} path {path:?} must be a non-empty forward-slash repository-relative path"
        ));
    }
    let path = Path::new(path);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "{label} path {} must not contain a prefix, root, or dot component",
            path.display()
        ));
    }
    for component in path.components() {
        let Component::Normal(component) = component else {
            return Err(format!(
                "{label} path {} must not contain a prefix, root, or dot component",
                path.display()
            ));
        };
        let component = component
            .to_str()
            .ok_or_else(|| format!("{label} path {} must be valid UTF-8", path.display()))?;
        validate_portable_path_component(component, label, path)?;
    }
    Ok(path)
}

fn validate_portable_path_component(
    component: &str,
    label: &str,
    path: &Path,
) -> Result<(), String> {
    let has_forbidden_character = component.bytes().any(|byte| {
        byte.is_ascii_control() || matches!(byte, b'<' | b'>' | b':' | b'"' | b'|' | b'?' | b'*')
    });
    if component.is_empty()
        || component != component.trim()
        || component.ends_with('.')
        || has_forbidden_character
        || is_windows_reserved_name(component)
    {
        return Err(format!(
            "{label} path {} contains a non-portable repository path component {component:?}",
            path.display()
        ));
    }
    Ok(())
}

fn is_windows_reserved_name(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .trim_end_matches([' ', '.']);
    let stem = stem.to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"))
        .is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
}

fn validate_exact_list(label: &str, actual: &[String], expected: &[&str]) -> Result<(), String> {
    let actual = actual.iter().map(String::as_str).collect::<Vec<_>>();
    if actual != expected {
        return Err(format!(
            "U4c lifecycle diagnostic {label} must equal {expected:?}, found {actual:?}"
        ));
    }
    Ok(())
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn normalized_sha256_hex(contents: &[u8], path: &Path) -> Result<String, String> {
    let contents = std::str::from_utf8(contents).map_err(|error| {
        format!(
            "U4c source anchor {} must be UTF-8 text before SHA-256 normalization: {error}",
            path.display()
        )
    })?;
    let normalized = contents.replace("\r\n", "\n");
    Ok(Sha256::digest(normalized.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn metadata_line<'a>(output: &'a str, key: &str) -> Result<&'a str, String> {
    let prefix = format!("{key}: ");
    let mut values = output.lines().filter_map(|line| line.strip_prefix(&prefix));
    let value = values
        .next()
        .ok_or_else(|| format!("Cargo-selected rustc probe did not emit {key:?}"))?;
    if values.next().is_some() {
        return Err(format!(
            "Cargo-selected rustc probe emitted duplicate {key:?} metadata"
        ));
    }
    Ok(value)
}

fn stderr_diagnostic(stderr: &[u8]) -> String {
    let diagnostic = String::from_utf8_lossy(stderr);
    let diagnostic = diagnostic.trim();
    if diagnostic.is_empty() {
        "no stderr was emitted".to_owned()
    } else {
        diagnostic.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_specification_has_only_lifecycle_diagnostic_scope() {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let specification_path = Path::new("docs/conformance/u4c-lifecycle-diagnostic.toml");
        assert_eq!(
            read_evidence_scope(&repository_root, specification_path),
            Ok(LIFECYCLE_DIAGNOSTIC_SCOPE.to_owned())
        );
        let specification = LifecycleDiagnosticSpec::load(&repository_root, specification_path)
            .expect("checked-in U4c lifecycle diagnostic specification and source anchors");

        assert_eq!(specification.evidence_scope(), LIFECYCLE_DIAGNOSTIC_SCOPE);
        assert_eq!(specification.fixture_id(), COMPLEX_FIXTURE_ID);
        assert!(is_lowercase_sha256(specification.specification_sha256()));
    }

    #[test]
    fn rejects_a_production_claim_from_the_lifecycle_schema() {
        let contents = include_str!("../../../../docs/conformance/u4c-lifecycle-diagnostic.toml")
            .replace("qualifies_u4_exit = false", "qualifies_u4_exit = true");

        let error = parse_specification(&contents).expect_err("production claim must reject");
        assert!(error.contains("qualifies_u4_exit"), "{error}");
    }

    #[test]
    fn recognizes_production_scope_only_for_fail_closed_routing() {
        assert_eq!(
            parse_evidence_scope("evidence_scope = \"u4c-production\"\nfuture_field = true\n"),
            Ok("u4c-production".to_owned())
        );
        assert!(parse_evidence_scope("evidence_scope = \"\"\n").is_err());
    }

    #[test]
    fn rejects_non_normal_anchor_paths_and_hashes() {
        assert!(validated_relative_path("../Cargo.toml", "anchor").is_err());
        assert!(validated_relative_path("crates\\baozi-core", "anchor").is_err());
        assert!(validated_relative_path("crates//baozi-core", "anchor").is_err());
        assert!(validated_relative_path("inputs/store.rs:payload", "anchor").is_err());
        assert!(validated_relative_path("inputs/CON.txt", "anchor").is_err());
        assert!(validated_relative_path("inputs/CON .txt", "anchor").is_err());
        assert!(validated_relative_path("inputs/COM¹.txt", "anchor").is_err());
        assert!(validated_relative_path("inputs/trailing. ", "anchor").is_err());
        assert!(validated_relative_path("inputs/control\u{1f}.rs", "anchor").is_err());
        assert!(validated_relative_path("crates/baozi-core", "anchor").is_ok());
        assert!(!is_lowercase_sha256("A"));
        assert!(is_lowercase_sha256(&"a".repeat(64)));
    }

    #[test]
    fn source_hash_normalizes_windows_line_endings() {
        let lf = normalized_sha256_hex(b"first\nsecond\n", Path::new("anchor"))
            .expect("LF source digest");
        let crlf = normalized_sha256_hex(b"first\r\nsecond\r\n", Path::new("anchor"))
            .expect("CRLF source digest");

        assert_eq!(lf, crlf);
    }

    #[test]
    fn toolchain_metadata_requires_exactly_one_value() {
        assert_eq!(
            metadata_line("release: 1.97.1\nhost: example\n", "release"),
            Ok("1.97.1")
        );
        assert!(metadata_line("host: example\n", "release").is_err());
        assert!(metadata_line("release: first\nrelease: second\n", "release").is_err());
    }
}
