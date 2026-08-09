//! Bounded checks for the handwritten repository conformance inventories.

mod evidence;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

/// Canonical contract inventory path.
pub const CONTRACTS_PATH: &str = "docs/conformance/contracts.toml";
/// Canonical executable gate catalog path.
pub const GATES_PATH: &str = "docs/conformance/gates.toml";

const SCHEMA_VERSION: u32 = 1;
const MAX_INVENTORY_BYTES: u64 = 256 * 1024;
const MAX_RECORDS: usize = 128;
const MAX_REFERENCES: usize = 64;

/// Result of checking the bounded repository conformance inventories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConformanceReport {
    /// Number of contract records parsed from the contract inventory.
    pub contracts_checked: usize,
    /// Number of executable gate records parsed from the gate catalog.
    pub gates_checked: usize,
    /// Number of records that declare `conforming` before trusted receipt admission.
    pub declared_conforming_contracts: usize,
    /// Number of declared conforming records matched by a self-attested evidence bundle.
    ///
    /// This measures only typed shape and exact binding consistency. It is never promotion
    /// authority and must remain separate from any future trusted receipt decision.
    pub self_attested_contract_bindings_matched: usize,
    /// Stable, human-readable contract violations.
    pub violations: Vec<String>,
}

/// Checks the two canonical conformance files and only their explicitly referenced paths.
pub fn check_repository(repository_root: &Path) -> ConformanceReport {
    let contracts = match read_explicit_file(repository_root, CONTRACTS_PATH, "contract inventory")
    {
        Ok(contents) => contents,
        Err(violation) => return failed_report(violation),
    };
    let gates = match read_explicit_file(repository_root, GATES_PATH, "gate catalog") {
        Ok(contents) => contents,
        Err(violation) => return failed_report(violation),
    };
    check_inventory_documents(repository_root, &contracts, &gates)
}

/// Checks one self-attested evidence bundle against the canonical inventory bytes.
///
/// A successful report proves only typed shape and exact binding consistency. It does not
/// establish command execution or authorize a contract promotion.
pub fn check_repository_evidence_bindings(
    repository_root: &Path,
    evidence_path: &Path,
    repository_revision: &str,
) -> ConformanceReport {
    let contracts = match read_explicit_file(repository_root, CONTRACTS_PATH, "contract inventory")
    {
        Ok(contents) => contents,
        Err(violation) => return failed_report(violation),
    };
    let gates = match read_explicit_file(repository_root, GATES_PATH, "gate catalog") {
        Ok(contents) => contents,
        Err(violation) => return failed_report(violation),
    };
    let Some(evidence_path) = evidence_path.to_str() else {
        return failed_report(
            "repository conformance: evidence bundle path must be valid UTF-8".to_owned(),
        );
    };
    let evidence = match read_explicit_file(repository_root, evidence_path, "evidence bundle") {
        Ok(contents) => contents,
        Err(violation) => return failed_report(violation),
    };
    check_inventory_evidence_bindings(
        repository_root,
        &contracts,
        &gates,
        &evidence,
        repository_revision,
    )
}

/// Checks supplied inventories against one self-attested typed evidence bundle.
///
/// The checker compares every required result with the supplied repository revision and raw
/// inventory bytes. The bundle is caller-controlled, no command or workflow is executed, and a
/// successful result cannot authorize conformance promotion.
pub fn check_inventory_evidence_bindings(
    repository_root: &Path,
    contracts_document: &str,
    gates_document: &str,
    evidence_document: &str,
    repository_revision: &str,
) -> ConformanceReport {
    evidence::check_inventory_evidence_bindings(
        repository_root,
        contracts_document,
        gates_document,
        evidence_document,
        repository_revision,
    )
}

/// Checks supplied inventory documents against explicit paths below `repository_root`.
///
/// This entry point exists so the dedicated conformance test target can exercise malformed
/// inventories without creating files or invoking repository tools.
pub fn check_inventory_documents(
    repository_root: &Path,
    contracts_document: &str,
    gates_document: &str,
) -> ConformanceReport {
    let mut report =
        check_inventory_document_structure(repository_root, contracts_document, gates_document);
    if report.declared_conforming_contracts != 0 {
        report.violations.push(
            "repository conformance: declared conforming contracts require a future trusted receipt adapter; handwritten catalogs and self-attested evidence cannot authorize promotion"
                .to_owned(),
        );
        report.violations.sort_unstable();
        report.violations.dedup();
    }
    report
}

pub(super) fn check_inventory_document_structure(
    repository_root: &Path,
    contracts_document: &str,
    gates_document: &str,
) -> ConformanceReport {
    if document_exceeds_limit(contracts_document) {
        return failed_report(format!(
            "repository conformance: {CONTRACTS_PATH} exceeds {MAX_INVENTORY_BYTES} bytes"
        ));
    }
    if document_exceeds_limit(gates_document) {
        return failed_report(format!(
            "repository conformance: {GATES_PATH} exceeds {MAX_INVENTORY_BYTES} bytes"
        ));
    }
    let contracts = match toml::from_str::<ContractInventory>(contracts_document) {
        Ok(inventory) => inventory,
        Err(error) => {
            return failed_report(format!(
                "repository conformance: failed to parse {CONTRACTS_PATH}: {error}"
            ));
        }
    };
    let declared_conforming_contracts = contracts
        .contracts
        .iter()
        .filter(|contract| contract.conformance_status == ConformanceStatus::Conforming)
        .count();
    let gates = match toml::from_str::<GateCatalog>(gates_document) {
        Ok(catalog) => catalog,
        Err(error) => {
            return ConformanceReport {
                contracts_checked: contracts.contracts.len(),
                gates_checked: 0,
                declared_conforming_contracts,
                self_attested_contract_bindings_matched: 0,
                violations: vec![format!(
                    "repository conformance: failed to parse {GATES_PATH}: {error}"
                )],
            };
        }
    };

    let mut violations = Vec::new();
    validate_inventories(repository_root, &contracts, &gates, &mut violations);
    violations.sort_unstable();
    violations.dedup();
    ConformanceReport {
        contracts_checked: contracts.contracts.len(),
        gates_checked: gates.gates.len(),
        declared_conforming_contracts,
        self_attested_contract_bindings_matched: 0,
        violations,
    }
}

fn document_exceeds_limit(document: &str) -> bool {
    u64::try_from(document.len()).map_or(true, |length| length > MAX_INVENTORY_BYTES)
}

fn failed_report(violation: String) -> ConformanceReport {
    ConformanceReport {
        contracts_checked: 0,
        gates_checked: 0,
        declared_conforming_contracts: 0,
        self_attested_contract_bindings_matched: 0,
        violations: vec![violation],
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractInventory {
    schema_version: u32,
    contracts: Vec<ContractRecord>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractRecord {
    contract_id: String,
    authority_adr: String,
    conformance_status: ConformanceStatus,
    evidence_subject: String,
    evidence_revision: u32,
    implementation_tracers: Vec<String>,
    test_tracers: Vec<String>,
    ci_tracers: Vec<String>,
    gate_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ConformanceStatus {
    Unassessed,
    Partial,
    Conforming,
    Nonconforming,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GateCatalog {
    schema_version: u32,
    gates: Vec<GateRecord>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GateRecord {
    gate_id: String,
    tracer_class: TracerClass,
    executor: GateExecutor,
    success_condition: GateSuccessCondition,
    evidence_identity: String,
    evidence_paths: Vec<String>,
    applicable_contracts: Vec<String>,
    #[serde(default)]
    evidence_claims: Vec<EvidenceClaim>,
    program: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    workflow_path: Option<String>,
    job_id: Option<String>,
    step_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
struct EvidenceClaim {
    contract_id: String,
    evidence_subject: String,
    evidence_revision: u32,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
enum TracerClass {
    Implementation,
    Test,
    Ci,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum GateExecutor {
    Command,
    Workflow,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum GateSuccessCondition {
    ProcessExitZero,
    RequiredWorkflowStatus,
}

fn validate_inventories(
    repository_root: &Path,
    contracts: &ContractInventory,
    gates: &GateCatalog,
    violations: &mut Vec<String>,
) {
    if contracts.schema_version != SCHEMA_VERSION {
        violations.push(format!(
            "repository conformance: {CONTRACTS_PATH} schema_version must be {SCHEMA_VERSION}"
        ));
    }
    if gates.schema_version != SCHEMA_VERSION {
        violations.push(format!(
            "repository conformance: {GATES_PATH} schema_version must be {SCHEMA_VERSION}"
        ));
    }
    if contracts.contracts.is_empty() || contracts.contracts.len() > MAX_RECORDS {
        violations.push(format!(
            "repository conformance: contract count must be in 1..={MAX_RECORDS}"
        ));
    }
    if gates.gates.is_empty() || gates.gates.len() > MAX_RECORDS {
        violations.push(format!(
            "repository conformance: gate count must be in 1..={MAX_RECORDS}"
        ));
    }

    let mut contracts_by_id = BTreeMap::new();
    for contract in &contracts.contracts {
        if !is_stable_id(&contract.contract_id) {
            violations.push(format!(
                "repository conformance: invalid contract_id {:?}",
                contract.contract_id
            ));
        }
        if contracts_by_id
            .insert(contract.contract_id.as_str(), contract)
            .is_some()
        {
            violations.push(format!(
                "repository conformance: duplicate contract_id {:?}",
                contract.contract_id
            ));
        }
        validate_contract(repository_root, contract, violations);
    }

    let mut gates_by_id = BTreeMap::new();
    for gate in &gates.gates {
        if !is_stable_id(&gate.gate_id) {
            violations.push(format!(
                "repository conformance: invalid gate_id {:?}",
                gate.gate_id
            ));
        }
        if gates_by_id.insert(gate.gate_id.as_str(), gate).is_some() {
            violations.push(format!(
                "repository conformance: duplicate gate_id {:?}",
                gate.gate_id
            ));
        }
        validate_gate(repository_root, gate, violations);
    }

    let mut referenced_gates = BTreeSet::new();
    for contract in &contracts.contracts {
        let mut referenced_classes = BTreeSet::new();
        let mut has_executable_gate = false;
        for gate_id in &contract.gate_ids {
            referenced_gates.insert(gate_id.as_str());
            let Some(gate) = gates_by_id.get(gate_id.as_str()) else {
                violations.push(format!(
                    "repository conformance: contract {:?} references unknown gate {:?}",
                    contract.contract_id, gate_id
                ));
                continue;
            };
            if !gate
                .applicable_contracts
                .iter()
                .any(|candidate| candidate == &contract.contract_id)
            {
                violations.push(format!(
                    "repository conformance: gate {:?} does not declare contract {:?} applicable",
                    gate.gate_id, contract.contract_id
                ));
            }
            referenced_classes.insert(gate.tracer_class);
            has_executable_gate |= gate.executor == GateExecutor::Command;
        }
        if contract.conformance_status == ConformanceStatus::Conforming {
            let has_all_tracers = !contract.implementation_tracers.is_empty()
                && !contract.test_tracers.is_empty()
                && !contract.ci_tracers.is_empty();
            let has_all_gate_classes = [
                TracerClass::Implementation,
                TracerClass::Test,
                TracerClass::Ci,
            ]
            .into_iter()
            .all(|class| referenced_classes.contains(&class));
            let has_exact_evidence_claim = contract.gate_ids.iter().any(|gate_id| {
                gates_by_id.get(gate_id.as_str()).is_some_and(|gate| {
                    gate.evidence_claims.iter().any(|claim| {
                        claim.contract_id == contract.contract_id
                            && claim.evidence_subject == contract.evidence_subject
                            && claim.evidence_revision == contract.evidence_revision
                    })
                })
            });
            if !has_all_tracers
                || !has_all_gate_classes
                || !has_executable_gate
                || !has_exact_evidence_claim
            {
                violations.push(format!(
                    "repository conformance: conforming contract {:?} lacks complete tracers, gate classes, executable evidence, or an exact evidence claim",
                    contract.contract_id
                ));
            }
        }
    }

    for gate in &gates.gates {
        if !referenced_gates.contains(gate.gate_id.as_str()) {
            violations.push(format!(
                "repository conformance: gate {:?} is not referenced by any contract",
                gate.gate_id
            ));
        }
        for contract_id in &gate.applicable_contracts {
            let Some(contract) = contracts_by_id.get(contract_id.as_str()) else {
                violations.push(format!(
                    "repository conformance: gate {:?} applies to unknown contract {:?}",
                    gate.gate_id, contract_id
                ));
                continue;
            };
            if !contract
                .gate_ids
                .iter()
                .any(|candidate| candidate == &gate.gate_id)
            {
                violations.push(format!(
                    "repository conformance: gate {:?} declares contract {:?} applicable, but the contract does not reference the gate",
                    gate.gate_id, contract_id
                ));
            }
        }
        for claim in &gate.evidence_claims {
            let Some(contract) = contracts_by_id.get(claim.contract_id.as_str()) else {
                violations.push(format!(
                    "repository conformance: gate {:?} claims evidence for unknown contract {:?}",
                    gate.gate_id, claim.contract_id
                ));
                continue;
            };
            if !gate
                .applicable_contracts
                .iter()
                .any(|candidate| candidate == &claim.contract_id)
            {
                violations.push(format!(
                    "repository conformance: gate {:?} evidence claim for contract {:?} is not applicable",
                    gate.gate_id, claim.contract_id
                ));
            }
            if claim.evidence_subject != contract.evidence_subject
                || claim.evidence_revision != contract.evidence_revision
            {
                violations.push(format!(
                    "repository conformance: gate {:?} evidence claim for contract {:?} does not match the contract evidence identity",
                    gate.gate_id, claim.contract_id
                ));
            }
        }
    }
}

fn validate_contract(
    repository_root: &Path,
    contract: &ContractRecord,
    violations: &mut Vec<String>,
) {
    if !contract.authority_adr.starts_with("docs/adr/") || !contract.authority_adr.ends_with(".md")
    {
        violations.push(format!(
            "repository conformance: contract {:?} authority_adr must be an explicit docs/adr path",
            contract.contract_id
        ));
    }
    validate_reference_path(
        repository_root,
        &contract.authority_adr,
        &format!("contract {:?} authority_adr", contract.contract_id),
        violations,
    );
    if !is_subject_id(&contract.evidence_subject) || contract.evidence_revision == 0 {
        violations.push(format!(
            "repository conformance: contract {:?} has an invalid evidence subject or revision",
            contract.contract_id
        ));
    }
    for (class, tracers) in [
        ("implementation", &contract.implementation_tracers),
        ("test", &contract.test_tracers),
        ("CI", &contract.ci_tracers),
    ] {
        validate_reference_list(
            repository_root,
            tracers,
            &format!("contract {:?} {class} tracer", contract.contract_id),
            violations,
        );
    }
    validate_id_list(
        &contract.gate_ids,
        &format!("contract {:?} gate_ids", contract.contract_id),
        violations,
    );
    if contract.gate_ids.is_empty() {
        violations.push(format!(
            "repository conformance: contract {:?} must reference at least one gate",
            contract.contract_id
        ));
    }
}

fn validate_gate(repository_root: &Path, gate: &GateRecord, violations: &mut Vec<String>) {
    validate_id_list(
        &gate.applicable_contracts,
        &format!("gate {:?} applicable_contracts", gate.gate_id),
        violations,
    );
    if gate.applicable_contracts.is_empty() {
        violations.push(format!(
            "repository conformance: gate {:?} must apply to at least one contract",
            gate.gate_id
        ));
    }
    if !is_evidence_identity(&gate.evidence_identity) {
        violations.push(format!(
            "repository conformance: gate {:?} has an invalid evidence_identity",
            gate.gate_id
        ));
    }
    validate_reference_list(
        repository_root,
        &gate.evidence_paths,
        &format!("gate {:?} evidence", gate.gate_id),
        violations,
    );
    if gate.evidence_paths.is_empty() {
        violations.push(format!(
            "repository conformance: gate {:?} must name explicit evidence paths",
            gate.gate_id
        ));
    }
    validate_evidence_claim_list(
        &gate.evidence_claims,
        &format!("gate {:?} evidence_claims", gate.gate_id),
        violations,
    );

    match gate.executor {
        GateExecutor::Command => {
            if gate.tracer_class == TracerClass::Ci
                || gate.success_condition != GateSuccessCondition::ProcessExitZero
                || gate
                    .program
                    .as_deref()
                    .is_none_or(|program| !is_nonblank(program))
                || gate.args.is_empty()
                || gate.workflow_path.is_some()
                || gate.job_id.is_some()
                || gate.step_name.is_some()
            {
                violations.push(format!(
                    "repository conformance: command gate {:?} must use program + args and process-exit-zero",
                    gate.gate_id
                ));
            }
        }
        GateExecutor::Workflow => {
            let workflow_path = gate.workflow_path.as_deref();
            if gate.tracer_class != TracerClass::Ci
                || gate.success_condition != GateSuccessCondition::RequiredWorkflowStatus
                || gate.program.is_some()
                || !gate.args.is_empty()
                || workflow_path.is_none_or(|path| !is_nonblank(path))
                || gate.job_id.as_deref().is_none_or(|job| !is_nonblank(job))
                || gate
                    .step_name
                    .as_deref()
                    .is_none_or(|step| !is_nonblank(step))
                || workflow_path.is_some_and(|path| {
                    !gate.evidence_paths.iter().any(|evidence| evidence == path)
                })
            {
                violations.push(format!(
                    "repository conformance: workflow gate {:?} must identify one CI workflow job and step",
                    gate.gate_id
                ));
            }
            if let Some(path) = &gate.workflow_path {
                validate_reference_path(
                    repository_root,
                    path,
                    &format!("gate {:?} workflow", gate.gate_id),
                    violations,
                );
            }
        }
    }
    if gate.args.len() > MAX_REFERENCES || gate.args.iter().any(|argument| argument.is_empty()) {
        violations.push(format!(
            "repository conformance: gate {:?} has invalid command arguments",
            gate.gate_id
        ));
    }
}

fn validate_evidence_claim_list(
    values: &[EvidenceClaim],
    label: &str,
    violations: &mut Vec<String>,
) {
    if values.len() > MAX_REFERENCES {
        violations.push(format!(
            "repository conformance: {label} exceeds {MAX_REFERENCES} entries"
        ));
    }
    let mut unique = BTreeSet::new();
    for value in values {
        if !is_stable_id(&value.contract_id) {
            violations.push(format!(
                "repository conformance: {label} contains invalid contract ID {:?}",
                value.contract_id
            ));
        }
        if !unique.insert(value.contract_id.as_str()) {
            violations.push(format!(
                "repository conformance: {label} contains duplicate contract {:?}",
                value.contract_id
            ));
        }
        if !is_subject_id(&value.evidence_subject) || value.evidence_revision == 0 {
            violations.push(format!(
                "repository conformance: {label} contains an invalid evidence claim for contract {:?}",
                value.contract_id
            ));
        }
    }
}

fn validate_id_list(values: &[String], label: &str, violations: &mut Vec<String>) {
    if values.len() > MAX_REFERENCES {
        violations.push(format!(
            "repository conformance: {label} exceeds {MAX_REFERENCES} entries"
        ));
    }
    let mut unique = BTreeSet::new();
    for value in values {
        if !is_stable_id(value) {
            violations.push(format!(
                "repository conformance: {label} contains invalid ID {value:?}"
            ));
        }
        if !unique.insert(value) {
            violations.push(format!(
                "repository conformance: {label} contains duplicate {value:?}"
            ));
        }
    }
}

fn validate_reference_list(
    repository_root: &Path,
    values: &[String],
    label: &str,
    violations: &mut Vec<String>,
) {
    if values.len() > MAX_REFERENCES {
        violations.push(format!(
            "repository conformance: {label} exceeds {MAX_REFERENCES} entries"
        ));
    }
    let mut unique = BTreeSet::new();
    for value in values {
        if !unique.insert(value.to_ascii_lowercase()) {
            violations.push(format!(
                "repository conformance: {label} contains a duplicate portable path alias {value:?}"
            ));
        }
        validate_reference_path(repository_root, value, label, violations);
    }
}

fn validate_reference_path(
    repository_root: &Path,
    value: &str,
    label: &str,
    violations: &mut Vec<String>,
) {
    if let Err(error) = explicit_file_path(repository_root, value) {
        violations.push(format!(
            "repository conformance: {label} path {value:?} {error}"
        ));
    }
}

fn read_explicit_file(repository_root: &Path, value: &str, label: &str) -> Result<String, String> {
    let path = explicit_file_path(repository_root, value)
        .map_err(|error| format!("repository conformance: {label} path {value:?} {error}"))?;
    let metadata = fs::metadata(&path).map_err(|error| {
        format!("repository conformance: cannot inspect {label} {value:?}: {error}")
    })?;
    if metadata.len() > MAX_INVENTORY_BYTES {
        return Err(format!(
            "repository conformance: {label} {value:?} exceeds {MAX_INVENTORY_BYTES} bytes"
        ));
    }
    fs::read_to_string(path)
        .map_err(|error| format!("repository conformance: cannot read {label} {value:?}: {error}"))
}

fn explicit_file_path(repository_root: &Path, value: &str) -> Result<PathBuf, &'static str> {
    validate_portable_reference_spelling(value)?;
    let relative = Path::new(value);
    let mut resolved = repository_root.to_path_buf();
    let mut components = relative.components().peekable();
    if components.peek().is_none() {
        return Err("must not be empty");
    }
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err("must not be absolute or contain dot/parent components");
        };
        resolved.push(name);
        let metadata = fs::symlink_metadata(&resolved).map_err(|_| "does not exist")?;
        if metadata.file_type().is_symlink() {
            return Err("must not traverse a symlink");
        }
        if components.peek().is_some() {
            if !metadata.is_dir() {
                return Err("has a non-directory parent component");
            }
        } else if !metadata.is_file() {
            return Err("must identify a regular file");
        }
    }
    Ok(resolved)
}

fn validate_portable_reference_spelling(value: &str) -> Result<(), &'static str> {
    const MAX_PATH_BYTES: usize = 512;

    if value.is_empty() || value.len() > MAX_PATH_BYTES || !value.is_ascii() || value.contains('\\')
    {
        return Err("must be a bounded ASCII forward-slash repository-relative path");
    }
    for segment in value.split('/') {
        if segment.is_empty() || matches!(segment, "." | "..") {
            return Err("must contain only nonempty normal components");
        }
        if segment.ends_with('.') || segment.ends_with(' ') {
            return Err("must not contain Windows-normalized trailing dot or space aliases");
        }
        if is_windows_reserved_component(segment) {
            return Err("must not contain a reserved Windows device name");
        }
        if segment.bytes().any(|byte| {
            byte.is_ascii_control()
                || matches!(byte, b'<' | b'>' | b':' | b'"' | b'|' | b'?' | b'*')
        }) {
            return Err("contains a non-portable path character");
        }
    }
    Ok(())
}

fn is_windows_reserved_component(segment: &str) -> bool {
    let stem = segment.split_once('.').map_or(segment, |(stem, _)| stem);
    if ["CON", "PRN", "AUX", "NUL", "CLOCK$", "CONIN$", "CONOUT$"]
        .into_iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
    {
        return true;
    }
    if stem.len() != 4 {
        return false;
    }
    let (prefix, suffix) = stem.split_at(3);
    (prefix.eq_ignore_ascii_case("COM") || prefix.eq_ignore_ascii_case("LPT"))
        && matches!(suffix.as_bytes(), [b'1'..=b'9'])
}

fn is_stable_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z'))
        && value.len() <= 96
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn is_subject_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'/')
        })
}

fn is_evidence_identity(value: &str) -> bool {
    is_nonblank(value) && value.len() <= 256 && !value.chars().any(char::is_control)
}

fn is_nonblank(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && !value.chars().all(char::is_whitespace)
}
