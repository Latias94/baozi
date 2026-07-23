//! U4c lifecycle diagnostics and measurement-scope admission.

mod spec;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::env;
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

pub(super) const PROBE_PREFIX: &str = "BAOZI_U4C_METRICS_V1=";
pub(super) const PROBE_TARGET: &str =
    "canonical_scene::store::tests::canonical_store_u4c_metrics_v1_probe";
const PROBE_ARGUMENTS: &[&str] = &[
    "test",
    "-j",
    "1",
    "--locked",
    "-p",
    "baozi-core",
    "--lib",
    PROBE_TARGET,
    "--",
    "--ignored",
    "--exact",
    "--nocapture",
    "--test-threads=1",
];

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MetricsV1 {
    schema_version: u64,
    fixture_schema_revision: u64,
    canonical_block_count: u64,
    outcome_evidence_block_count: u64,
    root_retained: bool,
    generation_reservations: u64,
    ledger: LedgerMetricsV1,
    physical_account: PhysicalAccountMetricsV1,
    factory_after_release: FactoryAfterReleaseV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LedgerMetricsV1 {
    terminal_success: bool,
    dimension_count: u64,
    phase_count: u64,
    current_live_bytes: u64,
    peak_live_bytes: u64,
    active_reservations: u64,
    current_evidence_bytes: u64,
    peak_evidence_bytes: u64,
    attempted: u64,
    consumed: u64,
    pending_publication: u64,
    committed: u64,
    committed_canonical_output: u64,
    published_outcome_evidence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PhysicalAccountMetricsV1 {
    before_prepare: PhysicalAccountSnapshotV1,
    prepared: PhysicalAccountSnapshotV1,
    after_handoff: PhysicalAccountSnapshotV1,
    after_release: PhysicalAccountSnapshotV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PhysicalAccountSnapshotV1 {
    live_retained_bytes: u64,
    live_retained_allocations: u64,
    peak_retained_bytes: u64,
    peak_retained_allocations: u64,
    total_admitted_retained_bytes: u64,
    total_admitted_retained_allocations: u64,
    poisoned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FactoryAfterReleaseV1 {
    live_accounts: u64,
    total_admitted_accounts: u64,
}

impl MetricsV1 {
    fn validate(&self) -> Result<(), String> {
        let physical_account = &self.physical_account;
        if physical_account.prepared != physical_account.after_handoff {
            return Err(
                "U4c metrics V1 physical_account.after_handoff must equal physical_account.prepared"
                    .to_owned(),
            );
        }

        let after_release = &physical_account.after_release;
        for (field, actual) in [
            ("live_retained_bytes", after_release.live_retained_bytes),
            (
                "live_retained_allocations",
                after_release.live_retained_allocations,
            ),
        ] {
            if actual != 0 {
                return Err(format!(
                    "U4c metrics V1 physical_account.after_release.{field} must be 0, found {actual}"
                ));
            }
        }
        if after_release.poisoned {
            return Err(
                "U4c metrics V1 physical_account.after_release.poisoned must be false".to_owned(),
            );
        }

        let factory_after_release = &self.factory_after_release;
        if factory_after_release.live_accounts != 0 {
            return Err(format!(
                "U4c metrics V1 factory_after_release.live_accounts must be 0, found {}",
                factory_after_release.live_accounts
            ));
        }

        if self.schema_version != 1 {
            return Err(format!(
                "U4c metrics V1 schema_version must be 1, found {}",
                self.schema_version
            ));
        }
        if !self.root_retained {
            return Err("U4c metrics V1 root_retained must be true".to_owned());
        }
        if self.generation_reservations != 1 {
            return Err(format!(
                "U4c metrics V1 generation_reservations must be 1, found {}",
                self.generation_reservations
            ));
        }

        let ledger = &self.ledger;
        if !ledger.terminal_success {
            return Err("U4c metrics V1 ledger.terminal_success must be true".to_owned());
        }
        for (field, actual) in [
            ("current_live_bytes", ledger.current_live_bytes),
            ("active_reservations", ledger.active_reservations),
            ("pending_publication", ledger.pending_publication),
        ] {
            if actual != 0 {
                return Err(format!(
                    "U4c metrics V1 ledger.{field} must be 0 after terminal handoff, found {actual}"
                ));
            }
        }

        if ledger.committed_canonical_output != self.canonical_block_count {
            return Err(format!(
                "U4c metrics V1 ledger.committed_canonical_output must equal canonical_block_count ({}), found {}",
                self.canonical_block_count, ledger.committed_canonical_output
            ));
        }

        let expected_published_outcome_evidence = self
            .outcome_evidence_block_count
            .checked_add(1)
            .ok_or_else(|| "U4c metrics V1 outcome evidence count overflowed".to_owned())?;
        if ledger.published_outcome_evidence != expected_published_outcome_evidence {
            return Err(format!(
                "U4c metrics V1 ledger.published_outcome_evidence must equal outcome_evidence_block_count + 1 ({expected_published_outcome_evidence}), found {}",
                ledger.published_outcome_evidence
            ));
        }

        Ok(())
    }
}

/// Runs the V1 lifecycle probe against a checked-in diagnostic specification.
///
/// This is intentionally not U4 exit evidence. The specification must state that scope explicitly.
pub fn collect_repository(
    repository_root: &Path,
    specification_path: &Path,
) -> Result<Value, String> {
    let evidence_scope = spec::read_evidence_scope(repository_root, specification_path)?;
    collect_evidence_scope(&evidence_scope, repository_root, specification_path)
}

fn collect_evidence_scope(
    evidence_scope: &str,
    repository_root: &Path,
    specification_path: &Path,
) -> Result<Value, String> {
    match evidence_scope {
        "lifecycle-diagnostic" => {
            collect_lifecycle_diagnostic(repository_root, specification_path)
        }
        "u4c-production" => Err(
            "U4c production specifications are not accepted until the sealed collector and exact closure discovery are implemented; production evidence cannot qualify U4 exit"
                .to_owned(),
        ),
        evidence_scope => Err(format!(
            "unsupported U4c evidence_scope {evidence_scope:?}; expected \"lifecycle-diagnostic\" or \"u4c-production\""
        )),
    }
}

fn collect_lifecycle_diagnostic(
    repository_root: &Path,
    specification_path: &Path,
) -> Result<Value, String> {
    let specification = spec::LifecycleDiagnosticSpec::load(repository_root, specification_path)?;
    specification.verify_toolchain(repository_root)?;
    let report = collect_v1_probe(repository_root)?;
    specification.verify_source_anchors(repository_root)?;
    specification.verify_report(&report)?;

    Ok(json!({
        "evidence_scope": specification.evidence_scope(),
        "qualifies_u4_exit": false,
        "specification_sha256": specification.specification_sha256(),
        "fixture": specification.fixture_id(),
        "metrics": report,
    }))
}

fn collect_v1_probe(repository_root: &Path) -> Result<MetricsV1, String> {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let output = Command::new(cargo)
        .args(PROBE_ARGUMENTS)
        .current_dir(repository_root)
        .output()
        .map_err(|error| format!("failed to run the U4c metrics probe: {error}"))?;

    if !output.status.success() {
        return Err(format!(
            "U4c metrics probe failed with {}: {}",
            output.status,
            stderr_diagnostic(&output.stderr)
        ));
    }

    let stdout = std::str::from_utf8(&output.stdout)
        .map_err(|error| format!("U4c metrics probe emitted non-UTF-8 stdout: {error}"))?;
    parse_probe_stdout_metrics(stdout)
}

/// Parses and validates the one V1 metrics report emitted by the core probe.
pub fn parse_probe_stdout(stdout: &str) -> Result<Value, String> {
    let report = parse_probe_stdout_metrics(stdout)?;
    serde_json::to_value(report)
        .map_err(|error| format!("U4c metrics probe could not serialize V1 metrics: {error}"))
}

fn parse_probe_stdout_metrics(stdout: &str) -> Result<MetricsV1, String> {
    let mut payload = None;
    let mut prefix_matches = 0usize;

    for line in stdout.lines() {
        if let Some(candidate) = line.strip_prefix(PROBE_PREFIX) {
            prefix_matches += 1;
            if prefix_matches == 1 {
                payload = Some(candidate);
            }
        }
    }

    let payload = match prefix_matches {
        0 => return Err("U4c metrics probe emitted no V1 metrics prefix".to_owned()),
        1 => payload.ok_or_else(|| "U4c metrics probe omitted V1 metrics payload".to_owned())?,
        _ => {
            return Err(format!(
                "U4c metrics probe emitted {prefix_matches} V1 metrics prefixes"
            ));
        }
    };

    let report = serde_json::from_str::<MetricsV1>(payload)
        .map_err(|error| format!("U4c metrics probe emitted invalid V1 JSON: {error}"))?;
    report.validate()?;
    Ok(report)
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
    use serde_json::json;

    #[test]
    fn parses_a_valid_report_after_harness_output() {
        let metrics = valid_metrics();
        let stdout = format!("test probe logs\n{PROBE_PREFIX}{metrics}\n");

        assert_eq!(parse_probe_stdout(&stdout), Ok(metrics));
    }

    #[test]
    fn production_scope_fails_before_any_probe_or_contract_claim() {
        let missing = Path::new("path-that-must-not-be-read");
        let error = collect_evidence_scope("u4c-production", missing, missing)
            .expect_err("production scope must fail closed");

        assert!(error.contains("not accepted"), "{error}");
        assert!(error.contains("cannot qualify U4 exit"), "{error}");
    }

    #[test]
    fn rejects_missing_or_multiple_prefixes() {
        assert!(parse_probe_stdout("test probe logs\n").is_err());

        let metrics = valid_metrics();
        let stdout = format!("{PROBE_PREFIX}{metrics}\n{PROBE_PREFIX}{metrics}\n");
        assert!(parse_probe_stdout(&stdout).is_err());
    }

    #[test]
    fn rejects_invalid_json() {
        assert!(parse_probe_stdout(&format!("{PROBE_PREFIX}not-json\n")).is_err());
    }

    #[test]
    fn rejects_mid_line_prefixes_and_duplicate_json_keys() {
        let metrics = valid_metrics();
        assert!(parse_probe_stdout(&format!("test logs {PROBE_PREFIX}{metrics}\n")).is_err());

        let duplicate_payload = metrics.to_string().replacen(
            "\"schema_version\":1",
            "\"schema_version\":0,\"schema_version\":1",
            1,
        );
        assert!(parse_probe_stdout(&format!("{PROBE_PREFIX}{duplicate_payload}\n")).is_err());

        let nested_duplicate_payload = metrics.to_string().replacen(
            "\"terminal_success\":true",
            "\"terminal_success\":false,\"terminal_success\":true",
            1,
        );
        assert!(
            parse_probe_stdout(&format!("{PROBE_PREFIX}{nested_duplicate_payload}\n")).is_err()
        );

        let snapshot_duplicate_payload = metrics.to_string().replacen(
            "\"live_retained_bytes\":0",
            "\"live_retained_bytes\":1,\"live_retained_bytes\":0",
            1,
        );
        assert!(
            parse_probe_stdout(&format!("{PROBE_PREFIX}{snapshot_duplicate_payload}\n")).is_err()
        );
    }

    #[test]
    fn rejects_unknown_or_missing_nested_keys() {
        let mut unknown = valid_metrics();
        unknown
            .get_mut("ledger")
            .and_then(Value::as_object_mut)
            .expect("valid ledger")
            .insert("unexpected".to_owned(), json!(0));
        assert!(parse_probe_stdout(&probe_stdout(&unknown)).is_err());

        let mut missing = valid_metrics();
        missing
            .get_mut("physical_account")
            .and_then(Value::as_object_mut)
            .and_then(|physical_account| physical_account.get_mut("after_release"))
            .and_then(Value::as_object_mut)
            .expect("valid after_release snapshot")
            .remove("poisoned");
        assert!(parse_probe_stdout(&probe_stdout(&missing)).is_err());
    }

    #[test]
    fn rejects_failed_value_semantics() {
        let mut metrics = valid_metrics();
        metrics["ledger"]["pending_publication"] = json!(1);

        assert!(parse_probe_stdout(&probe_stdout(&metrics)).is_err());

        let mut metrics = valid_metrics();
        metrics["root_retained"] = json!(false);
        assert!(parse_probe_stdout(&probe_stdout(&metrics)).is_err());

        let mut metrics = valid_metrics();
        metrics["generation_reservations"] = json!(0);
        assert!(parse_probe_stdout(&probe_stdout(&metrics)).is_err());
    }

    fn probe_stdout(metrics: &Value) -> String {
        format!("{PROBE_PREFIX}{metrics}\n")
    }

    fn valid_metrics() -> Value {
        let prepared = snapshot(8, 1, false);
        json!({
            "schema_version": 1,
            "fixture_schema_revision": 1,
            "canonical_block_count": 26,
            "outcome_evidence_block_count": 1,
            "root_retained": true,
            "generation_reservations": 1,
            "ledger": {
                "terminal_success": true,
                "dimension_count": 1,
                "phase_count": 1,
                "current_live_bytes": 0,
                "peak_live_bytes": 8,
                "active_reservations": 0,
                "current_evidence_bytes": 0,
                "peak_evidence_bytes": 1,
                "attempted": 1,
                "consumed": 1,
                "pending_publication": 0,
                "committed": 1,
                "committed_canonical_output": 26,
                "published_outcome_evidence": 2,
            },
            "physical_account": {
                "before_prepare": snapshot(0, 0, false),
                "prepared": prepared.clone(),
                "after_handoff": prepared,
                "after_release": snapshot(0, 0, false),
            },
            "factory_after_release": {
                "live_accounts": 0,
                "total_admitted_accounts": 1,
            },
        })
    }

    fn snapshot(live_retained_bytes: u64, live_retained_allocations: u64, poisoned: bool) -> Value {
        json!({
            "live_retained_bytes": live_retained_bytes,
            "live_retained_allocations": live_retained_allocations,
            "peak_retained_bytes": 8,
            "peak_retained_allocations": 1,
            "total_admitted_retained_bytes": 8,
            "total_admitted_retained_allocations": 1,
            "poisoned": poisoned,
        })
    }
}
