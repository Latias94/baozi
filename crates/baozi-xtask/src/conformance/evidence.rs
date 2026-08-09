//! Typed, fail-closed binding checks for self-attested repository gate evidence.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{
    ConformanceReport, ConformanceStatus, ContractInventory, EvidenceClaim, GateCatalog,
    GateExecutor, GateRecord, GateSuccessCondition, MAX_INVENTORY_BYTES, MAX_RECORDS,
    MAX_REFERENCES, SCHEMA_VERSION, check_inventory_document_structure, is_evidence_identity,
    is_stable_id, is_subject_id,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceBundle {
    schema_version: u32,
    trust_model: EvidenceTrustModel,
    gate_results: Vec<GateResult>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GateResult {
    gate_id: String,
    executor: GateExecutor,
    success_condition: GateSuccessCondition,
    evidence_identity: String,
    outcome: GateOutcome,
    repository_revision: String,
    contracts_sha256: String,
    gates_sha256: String,
    #[serde(default)]
    claims: Vec<EvidenceClaim>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum EvidenceTrustModel {
    SelfAttested,
    TrustedReceipt,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum GateOutcome {
    Passed,
    Failed,
}

pub(super) fn check_inventory_evidence_bindings(
    repository_root: &Path,
    contracts_document: &str,
    gates_document: &str,
    evidence_document: &str,
    repository_revision: &str,
) -> ConformanceReport {
    let mut report =
        check_inventory_document_structure(repository_root, contracts_document, gates_document);
    if u64::try_from(evidence_document.len()).unwrap_or(u64::MAX) > MAX_INVENTORY_BYTES {
        report.violations.push(format!(
            "repository conformance: evidence bundle exceeds {MAX_INVENTORY_BYTES} bytes"
        ));
        finish_report(&mut report, 0);
        return report;
    }
    if !report.violations.is_empty() {
        finish_report(&mut report, 0);
        return report;
    }

    let contracts = match toml::from_str::<ContractInventory>(contracts_document) {
        Ok(inventory) => inventory,
        Err(_) => {
            finish_report(&mut report, 0);
            return report;
        }
    };
    let gates = match toml::from_str::<GateCatalog>(gates_document) {
        Ok(catalog) => catalog,
        Err(_) => {
            finish_report(&mut report, 0);
            return report;
        }
    };
    let evidence = match toml::from_str::<EvidenceBundle>(evidence_document) {
        Ok(bundle) => bundle,
        Err(error) => {
            report.violations.push(format!(
                "repository conformance: failed to parse typed evidence bundle: {error}"
            ));
            finish_report(&mut report, 0);
            return report;
        }
    };

    let matched_bindings = check_evidence_bundle_bindings(
        &contracts,
        &gates,
        &evidence,
        contracts_document,
        gates_document,
        repository_revision,
        &mut report.violations,
    );
    finish_report(&mut report, matched_bindings);
    report
}

fn check_evidence_bundle_bindings(
    contracts: &ContractInventory,
    gates: &GateCatalog,
    evidence: &EvidenceBundle,
    contracts_document: &str,
    gates_document: &str,
    repository_revision: &str,
    violations: &mut Vec<String>,
) -> usize {
    if evidence.schema_version != SCHEMA_VERSION {
        violations.push(format!(
            "repository conformance: typed evidence schema_version must be {SCHEMA_VERSION}"
        ));
    }
    if evidence.trust_model != EvidenceTrustModel::SelfAttested {
        violations.push(
            "repository conformance: typed evidence trust_model must be self-attested".to_owned(),
        );
    }
    if evidence.gate_results.len() > MAX_RECORDS {
        violations.push(format!(
            "repository conformance: typed evidence gate result count must not exceed {MAX_RECORDS}"
        ));
    }
    if !is_repository_revision(repository_revision) {
        violations.push(
            "repository conformance: expected repository revision must be a complete lowercase hexadecimal Git object ID"
                .to_owned(),
        );
    }

    let contracts_sha256 = sha256_hex(contracts_document);
    let gates_sha256 = sha256_hex(gates_document);
    let gates_by_id = gates
        .gates
        .iter()
        .map(|gate| (gate.gate_id.as_str(), gate))
        .collect::<BTreeMap<_, _>>();
    let required_gate_ids = contracts
        .contracts
        .iter()
        .filter(|contract| contract.conformance_status == ConformanceStatus::Conforming)
        .flat_map(|contract| contract.gate_ids.iter().map(String::as_str))
        .collect::<BTreeSet<_>>();
    let mut results_by_id = BTreeMap::new();
    let mut binding_consistent_gate_results = BTreeSet::new();

    for result in &evidence.gate_results {
        if !is_stable_id(&result.gate_id) {
            violations.push(format!(
                "repository conformance: typed evidence contains invalid gate ID {:?}",
                result.gate_id
            ));
        }
        if results_by_id
            .insert(result.gate_id.as_str(), result)
            .is_some()
        {
            violations.push(format!(
                "repository conformance: duplicate gate result {:?}",
                result.gate_id
            ));
            continue;
        }
        let Some(gate) = gates_by_id.get(result.gate_id.as_str()).copied() else {
            violations.push(format!(
                "repository conformance: typed evidence references unknown gate {:?}",
                result.gate_id
            ));
            continue;
        };
        if !required_gate_ids.contains(result.gate_id.as_str()) {
            continue;
        }
        let violations_before = violations.len();
        validate_gate_result(
            result,
            gate,
            repository_revision,
            &contracts_sha256,
            &gates_sha256,
            violations,
        );
        if violations.len() == violations_before {
            binding_consistent_gate_results.insert(result.gate_id.as_str());
        }
    }

    for gate_id in &required_gate_ids {
        if !results_by_id.contains_key(gate_id) {
            violations.push(format!(
                "repository conformance: missing passing result for gate {:?}",
                gate_id
            ));
        }
    }

    let mut matched_contract_bindings = 0;
    for contract in contracts
        .contracts
        .iter()
        .filter(|contract| contract.conformance_status == ConformanceStatus::Conforming)
    {
        let exact_claim = EvidenceClaim {
            contract_id: contract.contract_id.clone(),
            evidence_subject: contract.evidence_subject.clone(),
            evidence_revision: contract.evidence_revision,
        };
        let all_referenced_gates_passed = contract
            .gate_ids
            .iter()
            .all(|gate_id| binding_consistent_gate_results.contains(gate_id.as_str()));
        let has_matching_claim = contract.gate_ids.iter().any(|gate_id| {
            binding_consistent_gate_results.contains(gate_id.as_str())
                && gates_by_id
                    .get(gate_id.as_str())
                    .is_some_and(|gate| gate.evidence_claims.contains(&exact_claim))
        });
        if all_referenced_gates_passed && has_matching_claim {
            matched_contract_bindings += 1;
        } else {
            violations.push(format!(
                "repository conformance: conforming contract {:?} is missing complete self-attested gate results or an exact evidence claim binding",
                contract.contract_id
            ));
        }
    }
    matched_contract_bindings
}

fn validate_gate_result(
    result: &GateResult,
    gate: &GateRecord,
    repository_revision: &str,
    contracts_sha256: &str,
    gates_sha256: &str,
    violations: &mut Vec<String>,
) {
    if result.outcome != GateOutcome::Passed {
        violations.push(format!(
            "repository conformance: gate {:?} did not pass",
            result.gate_id
        ));
    }
    if result.repository_revision != repository_revision {
        violations.push(format!(
            "repository conformance: gate {:?} repository revision does not match the requested revision",
            result.gate_id
        ));
    }
    if result.contracts_sha256 != contracts_sha256 {
        violations.push(format!(
            "repository conformance: gate {:?} contract inventory digest does not match",
            result.gate_id
        ));
    }
    if result.gates_sha256 != gates_sha256 {
        violations.push(format!(
            "repository conformance: gate {:?} gate catalog digest does not match",
            result.gate_id
        ));
    }
    if result.executor != gate.executor {
        violations.push(format!(
            "repository conformance: gate {:?} executor does not match the catalog",
            result.gate_id
        ));
    }
    if result.success_condition != gate.success_condition {
        violations.push(format!(
            "repository conformance: gate {:?} success condition does not match the catalog",
            result.gate_id
        ));
    }
    if result.evidence_identity != gate.evidence_identity
        || !is_evidence_identity(&result.evidence_identity)
    {
        violations.push(format!(
            "repository conformance: gate {:?} evidence identity does not match the catalog",
            result.gate_id
        ));
    }
    if !is_sha256(&result.contracts_sha256) || !is_sha256(&result.gates_sha256) {
        violations.push(format!(
            "repository conformance: gate {:?} contains an invalid catalog digest",
            result.gate_id
        ));
    }
    if !is_repository_revision(&result.repository_revision) {
        violations.push(format!(
            "repository conformance: gate {:?} contains an invalid repository revision",
            result.gate_id
        ));
    }
    if result.claims.len() > MAX_REFERENCES {
        violations.push(format!(
            "repository conformance: gate {:?} contains more than {MAX_REFERENCES} evidence claims",
            result.gate_id
        ));
    }

    let mut claimed_contracts = BTreeSet::new();
    let mut claims = BTreeSet::new();
    for claim in &result.claims {
        if !is_stable_id(&claim.contract_id)
            || !is_subject_id(&claim.evidence_subject)
            || claim.evidence_revision == 0
        {
            violations.push(format!(
                "repository conformance: gate {:?} contains an invalid evidence claim",
                result.gate_id
            ));
        }
        if !claimed_contracts.insert(claim.contract_id.as_str()) {
            violations.push(format!(
                "repository conformance: gate {:?} contains a duplicate evidence claim for contract {:?}",
                result.gate_id, claim.contract_id
            ));
        }
        claims.insert(claim);
    }
    let catalog_claims = gate.evidence_claims.iter().collect::<BTreeSet<_>>();
    if claims != catalog_claims {
        violations.push(format!(
            "repository conformance: gate {:?} evidence claim set does not match the catalog",
            result.gate_id
        ));
    }
}

fn finish_report(report: &mut ConformanceReport, matched_bindings: usize) {
    report.violations.sort_unstable();
    report.violations.dedup();
    report.self_attested_contract_bindings_matched = if report.violations.is_empty() {
        matched_bindings
    } else {
        0
    };
}

fn sha256_hex(document: &str) -> String {
    format!("{:x}", Sha256::digest(document.as_bytes()))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn is_repository_revision(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
