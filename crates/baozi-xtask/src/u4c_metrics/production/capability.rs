use serde::Deserialize;

use super::is_lowercase_sha256;
#[cfg(feature = "repository-input-candidate")]
use super::{AllocationReport, PhaseReport, PhysicalAccountReport};

pub(super) const BOUNDED_PREVIEW_SCHEMA_ID: &str = "baozi.canonical-scene.bounded-preview.v1";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct FixedVector {
    pub(super) schema_id: String,
    pub(super) observed_bytes: u64,
    pub(super) sha256: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct BoundedPreview {
    pub(super) schema_id: String,
    pub(super) bytes: u64,
    pub(super) sha256: String,
    pub(super) items_omitted_by_limit: u64,
    pub(super) bytes_truncated: bool,
    pub(super) items_truncated: bool,
}

impl BoundedPreview {
    pub(super) fn has_common_invariants(&self) -> bool {
        self.schema_id == BOUNDED_PREVIEW_SCHEMA_ID
            && self.bytes > 0
            && self.bytes <= 96
            && is_lowercase_sha256(&self.sha256)
            && self.items_omitted_by_limit > 0
            && (self.bytes_truncated || self.items_truncated)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ConstructionBudgetSpecification {
    pub(super) allocations: AllocationBudgetSpecification,
    pub(super) max_payload_copy_bytes: u64,
}

impl ConstructionBudgetSpecification {
    pub(super) fn is_well_formed(&self) -> bool {
        self.allocations.is_well_formed() && self.max_payload_copy_bytes > 0
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SnapshotBudgetSpecification {
    pub(super) allocations: AllocationBudgetSpecification,
    pub(super) completed_lookups: u64,
    pub(super) lookup_events: u64,
    pub(super) member_probes: u64,
    pub(super) max_lookup_events: u64,
    pub(super) max_member_probes_per_lookup: u64,
}

impl SnapshotBudgetSpecification {
    pub(super) fn is_well_formed(&self) -> bool {
        let Some(maximum_lookup_events) =
            self.completed_lookups.checked_mul(self.max_lookup_events)
        else {
            return false;
        };
        let Some(maximum_member_probes) = self
            .completed_lookups
            .checked_mul(self.max_member_probes_per_lookup)
        else {
            return false;
        };

        self.allocations.is_well_formed()
            && self.completed_lookups > 0
            && self.lookup_events >= self.completed_lookups
            && self.member_probes <= self.lookup_events
            && self.max_lookup_events > 0
            && self.max_lookup_events <= self.lookup_events
            && self.lookup_events <= maximum_lookup_events
            && self.max_member_probes_per_lookup > 0
            && self.max_member_probes_per_lookup <= self.member_probes
            && self.member_probes <= maximum_member_probes
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AllocationBudgetSpecification {
    pub(super) max_count_total: u64,
    pub(super) max_bytes_total: u64,
    pub(super) max_count_peak: u64,
    pub(super) max_bytes_peak: u64,
    pub(super) max_count_current: u64,
    pub(super) max_bytes_current: u64,
}

impl AllocationBudgetSpecification {
    pub(super) fn is_well_formed(self) -> bool {
        self.max_count_current > 0
            && self.max_count_current <= self.max_count_peak
            && self.max_count_peak <= self.max_count_total
            && self.max_bytes_current > 0
            && self.max_bytes_current <= self.max_bytes_peak
            && self.max_bytes_peak <= self.max_bytes_total
    }

    #[cfg(feature = "repository-input-candidate")]
    pub(super) fn contains(self, allocations: &AllocationReport) -> bool {
        let Ok(count_current) = u64::try_from(allocations.count_current) else {
            return false;
        };
        let Ok(bytes_current) = u64::try_from(allocations.bytes_current) else {
            return false;
        };
        count_current > 0
            && count_current <= allocations.count_max
            && allocations.count_max <= allocations.count_total
            && allocations.count_total <= self.max_count_total
            && allocations.count_max <= self.max_count_peak
            && count_current <= self.max_count_current
            && bytes_current > 0
            && bytes_current <= allocations.bytes_max
            && allocations.bytes_max <= allocations.bytes_total
            && allocations.bytes_total <= self.max_bytes_total
            && allocations.bytes_max <= self.max_bytes_peak
            && bytes_current <= self.max_bytes_current
    }
}

#[cfg(feature = "repository-input-candidate")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PhysicalLifecycleReport {
    pub(super) baseline: PhysicalAccountReport,
    pub(super) retained: PhysicalAccountReport,
    pub(super) after_inspection: PhysicalAccountReport,
    pub(super) after_snapshot: PhysicalAccountReport,
    pub(super) after_release: PhysicalAccountReport,
}

#[cfg(feature = "repository-input-candidate")]
pub(super) fn validate_fixed_vector_report(
    report: &FixedVector,
    expected: &FixedVector,
    label: &str,
) -> Result<(), String> {
    if report != expected || !is_lowercase_sha256(&report.sha256) {
        return Err(format!("{label} fixed-vector evidence drifted"));
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
pub(super) fn validate_bounded_preview_report(
    report: &BoundedPreview,
    expected: &BoundedPreview,
    label: &str,
) -> Result<(), String> {
    if report != expected || !report.has_common_invariants() {
        return Err(format!("{label} bounded preview evidence drifted"));
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
pub(super) fn validate_nonproducing_phase(report: &PhaseReport, label: &str) -> Result<(), String> {
    let taxonomy = &report.taxonomy;
    if taxonomy.semantic_cell_writes != 0
        || taxonomy.reference_writes != 0
        || taxonomy.producer_events != 0
        || taxonomy.payload_copy_bytes != 0
        || taxonomy.payload_backing_allocations != 0
        || taxonomy.block_begins != 0
        || taxonomy.block_seals != 0
        || taxonomy.slot_publications != 0
        || taxonomy.new_block_begins != 0
        || taxonomy.new_block_seals != 0
        || taxonomy.new_slot_publications != 0
        || taxonomy.inherited_block_begins != 0
        || taxonomy.inherited_block_seals != 0
        || taxonomy.inherited_slot_publications != 0
        || taxonomy.element_decodes != 0
    {
        return Err(format!("{label} performed a producer-side event"));
    }
    Ok(())
}

#[cfg(feature = "repository-input-candidate")]
pub(super) fn validate_allocation_lifecycle<const PHASES: usize>(
    reports: [&AllocationReport; PHASES],
    label: &str,
) -> Result<(), String> {
    if reports.is_empty() {
        return Err(format!("{label} has no allocation lifecycle phases"));
    }
    let count_current = reports.iter().try_fold(0_i64, |total, report| {
        total.checked_add(report.count_current)
    });
    let bytes_current = reports.iter().try_fold(0_i64, |total, report| {
        total.checked_add(report.bytes_current)
    });
    if count_current != Some(0) || bytes_current != Some(0) {
        return Err(format!("{label} did not balance to zero"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exact_allocation_budget() -> AllocationBudgetSpecification {
        AllocationBudgetSpecification {
            max_count_total: 3,
            max_bytes_total: 9,
            max_count_peak: 3,
            max_bytes_peak: 9,
            max_count_current: 2,
            max_bytes_current: 8,
        }
    }

    fn exact_snapshot_budget() -> SnapshotBudgetSpecification {
        SnapshotBudgetSpecification {
            allocations: exact_allocation_budget(),
            completed_lookups: 2,
            lookup_events: 8,
            member_probes: 6,
            max_lookup_events: 4,
            max_member_probes_per_lookup: 3,
        }
    }

    #[cfg(feature = "repository-input-candidate")]
    #[test]
    fn rejects_an_empty_allocation_lifecycle() {
        let error = validate_allocation_lifecycle([], "empty lifecycle")
            .expect_err("an empty lifecycle must not balance vacuously");

        assert!(error.contains("no allocation lifecycle phases"), "{error}");
    }

    #[test]
    fn rejects_internally_impossible_snapshot_taxonomy() {
        let exact = exact_snapshot_budget();
        assert!(exact.is_well_formed());

        let impossible = [
            SnapshotBudgetSpecification {
                max_lookup_events: 9,
                ..exact_snapshot_budget()
            },
            SnapshotBudgetSpecification {
                max_member_probes_per_lookup: 7,
                ..exact_snapshot_budget()
            },
            SnapshotBudgetSpecification {
                lookup_events: 9,
                ..exact_snapshot_budget()
            },
            SnapshotBudgetSpecification {
                member_probes: 7,
                ..exact_snapshot_budget()
            },
        ];

        assert!(impossible.iter().all(|budget| !budget.is_well_formed()));
    }

    #[cfg(feature = "repository-input-candidate")]
    #[test]
    fn rejects_phase_allocation_budget_overrun() {
        let allocations = AllocationReport {
            count_total: 3,
            count_current: 2,
            count_max: 3,
            bytes_total: 9,
            bytes_current: 8,
            bytes_max: 9,
        };
        let exact = exact_allocation_budget();

        assert!(
            !AllocationBudgetSpecification {
                max_count_total: 2,
                ..exact
            }
            .contains(&allocations)
        );
        assert!(
            !AllocationBudgetSpecification {
                max_bytes_total: 8,
                ..exact
            }
            .contains(&allocations)
        );
        assert!(
            !AllocationBudgetSpecification {
                max_count_peak: 2,
                ..exact
            }
            .contains(&allocations)
        );
        assert!(
            !AllocationBudgetSpecification {
                max_bytes_peak: 8,
                ..exact
            }
            .contains(&allocations)
        );
        assert!(
            !AllocationBudgetSpecification {
                max_count_current: 1,
                ..exact
            }
            .contains(&allocations)
        );
        assert!(
            !AllocationBudgetSpecification {
                max_bytes_current: 7,
                ..exact
            }
            .contains(&allocations)
        );
        assert!(exact.contains(&allocations));
        assert!(exact.is_well_formed());

        let impossible = AllocationReport {
            count_total: 1,
            count_current: 1,
            count_max: 0,
            bytes_total: 1,
            bytes_current: 1,
            bytes_max: 0,
        };
        assert!(!exact.contains(&impossible));
    }

    #[cfg(feature = "repository-input-candidate")]
    #[test]
    fn rejects_unbalanced_phase_allocation_deltas() {
        fn delta(count_current: i64, bytes_current: i64) -> AllocationReport {
            AllocationReport {
                count_total: 0,
                count_current,
                count_max: 0,
                bytes_total: 0,
                bytes_current,
                bytes_max: 0,
            }
        }

        let construction = delta(37, 8_644);
        let inspection = delta(0, 0);
        let snapshot = delta(2, 8_288);
        let balanced_release = delta(-39, -16_932);
        let unbalanced_release = delta(-38, -16_932);

        assert!(
            validate_allocation_lifecycle(
                [&construction, &inspection, &snapshot, &balanced_release],
                "balanced",
            )
            .is_ok()
        );
        assert!(
            validate_allocation_lifecycle(
                [&construction, &inspection, &snapshot, &unbalanced_release],
                "unbalanced",
            )
            .is_err()
        );
    }
}
