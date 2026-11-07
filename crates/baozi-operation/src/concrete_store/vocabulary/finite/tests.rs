use std::error::Error;
use std::num::{NonZeroU32, NonZeroU64};

use super::super::{
    FiniteStoreLedgerPlan, FiniteStoreLedgerPlanError, StoreDimensionSemanticId,
    StorePhaseSemanticId, StoreSemanticCatalogId,
};
use super::{
    DimensionPolicy, FiniteLedgerLimits, LedgerLimitsError, RetainedStoreFailureTerminalWorkPlan,
    StoreDimensionKey, StoreKeyDomain, StoreLedgerPlanError, StorePhaseKey,
};

fn vocabulary() -> (StoreKeyDomain, [StoreDimensionKey; 6], [StorePhaseKey; 3]) {
    let catalog = StoreSemanticCatalogId::new(
        *b"baozi-unit-store",
        NonZeroU32::new(1).expect("nonzero catalog revision"),
    );
    let domain = StoreKeyDomain::try_new(catalog).expect("finite store domain");
    let dimensions = [1, 2, 3, 4, 5, 6].map(|local| {
        domain
            .try_admit_dimension(StoreDimensionSemanticId::new(
                catalog,
                NonZeroU64::new(local).expect("nonzero dimension identity"),
            ))
            .expect("admitted finite dimension")
    });
    let phases = [1, 2, 3].map(|local| {
        domain
            .try_admit_phase(StorePhaseSemanticId::new(
                catalog,
                NonZeroU64::new(local).expect("nonzero phase identity"),
            ))
            .expect("admitted finite phase")
    });
    (domain, dimensions, phases)
}

fn policies(dimensions: [StoreDimensionKey; 6]) -> [(StoreDimensionKey, DimensionPolicy); 6] {
    dimensions.map(|dimension| {
        let local = dimension.semantic_id().local().get();
        (
            dimension,
            DimensionPolicy::reservable_bytes(local * 2, local, local * 3, local * 4)
                .with_published_outcome_evidence_limit(local * 5),
        )
    })
}

fn failure_terminal_work(dimension: StoreDimensionKey) -> RetainedStoreFailureTerminalWorkPlan {
    let requirement = RetainedStoreFailureTerminalWorkPlan::requirement_for_evidence_work(2, 3, 5)
        .expect("finite terminal requirement");
    RetainedStoreFailureTerminalWorkPlan::with_evidence_work(
        dimension,
        2,
        3,
        5,
        requirement.attempted_work_limit() + 30,
        requirement.consumed_work_limit() + 15,
    )
    .expect("finite terminal work partition")
}

fn terminal_policies(
    dimensions: [StoreDimensionKey; 6],
    terminal: &RetainedStoreFailureTerminalWorkPlan,
) -> [(StoreDimensionKey, DimensionPolicy); 6] {
    let mut policies = policies(dimensions);
    policies[5].1 = terminal.requirement().dimension_policy();
    policies
}

#[test]
fn finite_plan_preserves_the_original_domain_and_exact_fixed_catalogs() {
    let (domain, dimensions, phases) = vocabulary();
    let original_id = domain.id;
    let catalog = domain.catalog();
    let unlisted_dimension = domain
        .try_admit_dimension(StoreDimensionSemanticId::new(
            catalog,
            NonZeroU64::new(7).expect("unlisted dimension identity"),
        ))
        .expect("unlisted owned dimension");
    let unlisted_phase = domain
        .try_admit_phase(StorePhaseSemanticId::new(
            catalog,
            NonZeroU64::new(4).expect("unlisted phase identity"),
        ))
        .expect("unlisted owned phase");
    let mut input = policies(dimensions);
    input.rotate_left(2);
    let plan = FiniteStoreLedgerPlan::try_new(domain, 128, 0, input, phases)
        .expect("allocation-free finite plan");

    assert_eq!(plan.dimension_count(), 6);
    assert_eq!(plan.phase_count(), 3);
    for dimension in dimensions {
        assert!(plan.owns_dimension(dimension));
        assert!(plan.contains_dimension(dimension));
    }
    for phase in phases {
        assert!(plan.owns_phase(phase));
        assert!(plan.contains_phase(phase));
    }
    assert!(plan.owns_dimension(unlisted_dimension));
    assert!(!plan.contains_dimension(unlisted_dimension));
    assert!(plan.owns_phase(unlisted_phase));
    assert!(!plan.contains_phase(unlisted_phase));

    let (domain, limits, frozen_phases): (
        StoreKeyDomain,
        FiniteLedgerLimits<StoreDimensionKey>,
        [StorePhaseKey; 3],
    ) = plan.into_parts();
    assert_eq!(domain.id, original_id);
    assert_eq!(domain.catalog(), catalog);
    assert_eq!(limits.dimensions(), &dimensions);
    for (dimension, policy) in input {
        assert_eq!(limits.dimension(dimension), Some(policy));
    }
    assert_eq!(limits.dimension(unlisted_dimension), None);
    assert_eq!(frozen_phases, phases);
}

#[test]
fn same_semantics_from_another_domain_never_match_a_finite_plan() {
    let (domain, dimensions, phases) = vocabulary();
    let (_foreign_domain, foreign_dimensions, foreign_phases) = vocabulary();
    let plan = FiniteStoreLedgerPlan::try_new(domain, 128, 0, policies(dimensions), phases)
        .expect("finite plan");

    for (owned, foreign) in dimensions.into_iter().zip(foreign_dimensions) {
        assert_eq!(owned.semantic_id(), foreign.semantic_id());
        assert!(!plan.owns_dimension(foreign));
        assert!(!plan.contains_dimension(foreign));
    }
    for (owned, foreign) in phases.into_iter().zip(foreign_phases) {
        assert_eq!(owned.semantic_id(), foreign.semantic_id());
        assert!(!plan.owns_phase(foreign));
        assert!(!plan.contains_phase(foreign));
    }
}

#[test]
fn finite_plan_checks_the_domain_of_every_dimension_and_phase() {
    let (_foreign_domain, foreign_dimensions, foreign_phases) = vocabulary();
    for position in 0..6 {
        let (domain, dimensions, phases) = vocabulary();
        let mut input = policies(dimensions);
        input[position].0 = foreign_dimensions[position];
        let error = FiniteStoreLedgerPlan::try_new(domain, 128, 0, input, phases)
            .expect_err("foreign dimension must be rejected");
        let cause = StoreLedgerPlanError::ForeignDimensionDomain {
            dimension: foreign_dimensions[position],
        };
        assert_eq!(
            error
                .source()
                .and_then(|source| source.downcast_ref::<StoreLedgerPlanError>()),
            Some(&cause),
        );
        assert_eq!(error, FiniteStoreLedgerPlanError::Catalog { cause });
    }
    for position in 0..3 {
        let (domain, dimensions, mut phases) = vocabulary();
        phases[position] = foreign_phases[position];
        assert_eq!(
            FiniteStoreLedgerPlan::try_new(domain, 128, 0, policies(dimensions), phases)
                .expect_err("foreign phase must be rejected"),
            FiniteStoreLedgerPlanError::Catalog {
                cause: StoreLedgerPlanError::ForeignPhaseDomain {
                    phase: foreign_phases[position],
                },
            },
        );
    }
}

#[test]
fn finite_dimension_domain_errors_follow_sorted_keys_and_precede_phase_errors() {
    let (domain, dimensions, phases) = vocabulary();
    let (_foreign_domain, foreign_dimensions, foreign_phases) = vocabulary();
    let mut input = policies(dimensions);
    input[0].0 = foreign_dimensions[5];
    input[5].0 = foreign_dimensions[0];
    assert_eq!(
        FiniteStoreLedgerPlan::try_new(
            domain,
            128,
            0,
            input,
            [phases[1], phases[0], foreign_phases[2]],
        )
        .expect_err("dimension domain validation precedes every phase check"),
        FiniteStoreLedgerPlanError::Catalog {
            cause: StoreLedgerPlanError::ForeignDimensionDomain {
                dimension: foreign_dimensions[0],
            },
        },
    );
}

#[test]
fn finite_phase_domain_errors_precede_pair_errors_and_follow_input_order() {
    let (_foreign_domain, _, foreign_phases) = vocabulary();
    for case in 0..3 {
        let (domain, dimensions, phases) = vocabulary();
        let (input, foreign) = match case {
            0 => ([phases[0], phases[0], foreign_phases[2]], foreign_phases[2]),
            1 => ([phases[1], phases[0], foreign_phases[2]], foreign_phases[2]),
            2 => (
                [foreign_phases[2], phases[0], foreign_phases[0]],
                foreign_phases[2],
            ),
            _ => unreachable!(),
        };
        assert_eq!(
            FiniteStoreLedgerPlan::try_new(domain, 128, 0, policies(dimensions), input)
                .expect_err("all phase domains are checked before phase pairs"),
            FiniteStoreLedgerPlanError::Catalog {
                cause: StoreLedgerPlanError::ForeignPhaseDomain { phase: foreign },
            },
        );
    }
}

#[test]
fn finite_phase_duplicates_and_order_errors_keep_the_dynamic_pair_semantics() {
    for (order, previous, phase) in [
        ([0, 0, 1], None, 0),
        ([0, 1, 1], None, 1),
        ([1, 0, 2], Some(1), 0),
        ([0, 2, 1], Some(2), 1),
        ([0, 1, 0], Some(1), 0),
        ([1, 1, 0], None, 1),
        ([1, 0, 0], Some(1), 0),
    ] {
        let (domain, dimensions, phases) = vocabulary();
        let cause = match previous {
            Some(previous) => StoreLedgerPlanError::PhaseOutOfOrder {
                previous: phases[previous],
                phase: phases[phase],
            },
            None => StoreLedgerPlanError::DuplicatePhase {
                phase: phases[phase],
            },
        };
        assert_eq!(
            FiniteStoreLedgerPlan::try_new(
                domain,
                128,
                0,
                policies(dimensions),
                order.map(|position| phases[position]),
            )
            .expect_err("finite phases must arrive in strict canonical order"),
            FiniteStoreLedgerPlanError::Catalog { cause },
        );
    }
}

#[test]
fn finite_policy_errors_keep_input_order_and_precede_domain_and_phase_errors() {
    let (_foreign_domain, foreign_dimensions, foreign_phases) = vocabulary();
    for case in 0..3 {
        let (domain, dimensions, phases) = vocabulary();
        let mut input = policies(dimensions);
        input.reverse();
        let first = input[0].0;
        let cause = match case {
            0 => {
                input[0].1 = DimensionPolicy::counter(1, 2, 0);
                input[5].1 = DimensionPolicy::counter(1, 2, 0);
                LedgerLimitsError::ConsumedLimitExceedsAttempted {
                    dimension: first,
                    consumed_limit: 2,
                    attempted_limit: 1,
                }
            }
            1 => {
                input[1] = (first, DimensionPolicy::counter(1, 2, 0));
                LedgerLimitsError::ConsumedLimitExceedsAttempted {
                    dimension: first,
                    consumed_limit: 2,
                    attempted_limit: 1,
                }
            }
            2 => {
                input[1] = (first, DimensionPolicy::counter(2, 2, 0));
                input[2].1 = DimensionPolicy::counter(1, 2, 0);
                LedgerLimitsError::DuplicateDimension { dimension: first }
            }
            _ => unreachable!(),
        };
        input[3].0 = foreign_dimensions[3];
        let error = FiniteStoreLedgerPlan::try_new(
            domain,
            128,
            0,
            input,
            [phases[0], phases[0], foreign_phases[2]],
        )
        .expect_err("policy validation precedes domain validation and dimension sorting");
        assert_eq!(
            error
                .source()
                .and_then(|source| source.downcast_ref::<LedgerLimitsError<StoreDimensionKey>>()),
            Some(&cause),
        );
        assert_eq!(error, FiniteStoreLedgerPlanError::Limits { cause });
    }
}

#[test]
fn failed_finite_construction_burns_the_original_domain_keys() {
    for policy_failure in [false, true] {
        let (domain, dimensions, phases) = vocabulary();
        let mut input = policies(dimensions);
        if policy_failure {
            input[1].0 = input[0].0;
        }
        let error = FiniteStoreLedgerPlan::try_new(
            domain,
            128,
            0,
            input,
            [phases[0], phases[0], phases[2]],
        )
        .expect_err("the only authority is consumed by a rejected attempt");
        if policy_failure {
            assert_eq!(
                error,
                FiniteStoreLedgerPlanError::Limits {
                    cause: LedgerLimitsError::DuplicateDimension {
                        dimension: dimensions[0],
                    },
                },
            );
        } else {
            assert_eq!(
                error,
                FiniteStoreLedgerPlanError::Catalog {
                    cause: StoreLedgerPlanError::DuplicatePhase { phase: phases[0] },
                },
            );
        }

        let (fresh_domain, _, fresh_phases) = vocabulary();
        assert_eq!(
            FiniteStoreLedgerPlan::try_new(
                fresh_domain,
                128,
                0,
                policies(dimensions),
                fresh_phases,
            )
            .expect_err("fresh authority cannot reuse burned dimension keys"),
            FiniteStoreLedgerPlanError::Catalog {
                cause: StoreLedgerPlanError::ForeignDimensionDomain {
                    dimension: dimensions[0],
                },
            },
        );

        let (fresh_domain, fresh_dimensions, _) = vocabulary();
        assert_eq!(
            FiniteStoreLedgerPlan::try_new(
                fresh_domain,
                128,
                0,
                policies(fresh_dimensions),
                phases,
            )
            .expect_err("fresh authority cannot reuse burned phase keys"),
            FiniteStoreLedgerPlanError::Catalog {
                cause: StoreLedgerPlanError::ForeignPhaseDomain { phase: phases[0] },
            },
        );
    }
}

#[test]
fn finite_terminal_budget_matches_the_exact_protected_and_ordinary_partition() {
    let (domain, dimensions, phases) = vocabulary();
    let terminal = failure_terminal_work(dimensions[5]);
    let mut input = terminal_policies(dimensions, &terminal);
    input.rotate_right(1);
    let plan = FiniteStoreLedgerPlan::try_new(domain, 128, 0, input, phases)
        .expect("finite terminal budget");
    assert!(plan.failure_terminal_budget_matches(&terminal));
}

#[test]
fn finite_terminal_budget_requires_both_ordinary_totals_to_match_exactly() {
    for (attempted, consumed) in [(1, 1), (3, 1), (2, 0), (2, 2)] {
        let (domain, dimensions, phases) = vocabulary();
        let terminal = failure_terminal_work(dimensions[5]);
        let mut input = terminal_policies(dimensions, &terminal);
        input[0].1 = DimensionPolicy::counter(attempted, consumed, 0);
        let plan = FiniteStoreLedgerPlan::try_new(domain, 128, 0, input, phases)
            .expect("individually valid ordinary policies");
        assert!(!plan.failure_terminal_budget_matches(&terminal));
    }
}

#[test]
fn finite_terminal_budget_rejects_every_nonexact_protected_policy() {
    let requirement = RetainedStoreFailureTerminalWorkPlan::requirement_for_evidence_work(2, 3, 5)
        .expect("exact terminal requirement");
    let attempted = requirement.attempted_work_limit();
    let consumed = requirement.consumed_work_limit();
    let smaller_requirement =
        RetainedStoreFailureTerminalWorkPlan::requirement_for_evidence_work(2, 3, 4)
            .expect("different terminal requirement");
    for protected_policy in [
        DimensionPolicy::counter(attempted + 1, consumed, 0),
        DimensionPolicy::counter(attempted, consumed - 1, 0),
        DimensionPolicy::counter(attempted, consumed, 1),
        DimensionPolicy::reservable_bytes(attempted, consumed, 0, 0),
        DimensionPolicy::counter(attempted, consumed, 0).with_published_outcome_evidence_limit(1),
        smaller_requirement.dimension_policy(),
    ] {
        let (domain, dimensions, phases) = vocabulary();
        let terminal = failure_terminal_work(dimensions[5]);
        let mut input = terminal_policies(dimensions, &terminal);
        input[5].1 = protected_policy;
        let plan = FiniteStoreLedgerPlan::try_new(domain, 128, 0, input, phases)
            .expect("individually valid protected policy");
        assert!(!plan.failure_terminal_budget_matches(&terminal));
    }
}

#[test]
fn finite_terminal_budget_rejects_missing_and_foreign_terminal_dimensions() {
    let (domain, dimensions, phases) = vocabulary();
    let (_foreign_domain, foreign_dimensions, _) = vocabulary();
    let unlisted = domain
        .try_admit_dimension(StoreDimensionSemanticId::new(
            domain.catalog(),
            NonZeroU64::new(7).expect("unlisted terminal dimension identity"),
        ))
        .expect("owned but unlisted terminal dimension");
    let terminal = failure_terminal_work(dimensions[5]);
    let plan = FiniteStoreLedgerPlan::try_new(
        domain,
        128,
        0,
        terminal_policies(dimensions, &terminal),
        phases,
    )
    .expect("finite terminal budget");
    assert!(!plan.failure_terminal_budget_matches(&failure_terminal_work(unlisted)));
    assert!(plan.owns_dimension(unlisted));
    assert!(!plan.contains_dimension(unlisted));
    assert_eq!(
        dimensions[5].semantic_id(),
        foreign_dimensions[5].semantic_id(),
    );
    assert!(!plan.failure_terminal_budget_matches(&failure_terminal_work(foreign_dimensions[5])));
}

#[test]
fn finite_terminal_budget_rejects_checked_ordinary_sum_overflow() {
    let (domain, dimensions, phases) = vocabulary();
    let terminal = failure_terminal_work(dimensions[5]);
    let mut input = terminal_policies(dimensions, &terminal);
    input[0].1 = DimensionPolicy::counter(u64::MAX, u64::MAX, 0);
    input[1].1 = DimensionPolicy::counter(1, 1, 0);
    let plan = FiniteStoreLedgerPlan::try_new(domain, 128, 0, input, phases)
        .expect("individually valid policies with overflowing ordinary totals");
    assert!(!plan.failure_terminal_budget_matches(&terminal));
}

#[test]
fn finite_success_and_rejection_leave_the_real_ledger_allocation_failpoint_armed() {
    use crate::ledger::{
        LedgerEvidenceError, LedgerEvidencePurpose, LedgerLimits, test_evidence_allocation_guard,
        test_fail_next_evidence_allocation,
    };

    let _guard = test_evidence_allocation_guard();
    for case in 0..3 {
        let (domain, dimensions, mut phases) = vocabulary();
        let mut input = policies(dimensions);
        match case {
            0 => {}
            1 => phases[1] = phases[0],
            2 => input[1].1 = DimensionPolicy::counter(1, 2, 0),
            _ => unreachable!(),
        }
        test_fail_next_evidence_allocation();
        let result = FiniteStoreLedgerPlan::try_new(domain, 128, 0, input, phases);
        match case {
            0 => {
                let plan =
                    result.expect("finite limits and phases need no dynamic evidence budget");
                let (_, limits, frozen_phases): (
                    StoreKeyDomain,
                    FiniteLedgerLimits<StoreDimensionKey>,
                    [StorePhaseKey; 3],
                ) = plan.into_parts();
                assert_eq!(limits.dimensions(), &dimensions);
                assert_eq!(frozen_phases, phases);
            }
            1 => assert_eq!(
                result.expect_err("duplicate phases do not allocate"),
                FiniteStoreLedgerPlanError::Catalog {
                    cause: StoreLedgerPlanError::DuplicatePhase { phase: phases[0] },
                },
            ),
            2 => assert_eq!(
                result.expect_err("invalid policies do not allocate"),
                FiniteStoreLedgerPlanError::Limits {
                    cause: LedgerLimitsError::ConsumedLimitExceedsAttempted {
                        dimension: dimensions[1],
                        consumed_limit: 2,
                        attempted_limit: 1,
                    },
                },
            ),
            _ => unreachable!(),
        }

        let control_probe = LedgerLimits::new(128, [(0_u8, DimensionPolicy::counter(1, 1, 0))]);
        assert!(matches!(
            control_probe,
            Err(LedgerLimitsError::Evidence {
                cause: LedgerEvidenceError::AllocationFailed {
                    purpose: LedgerEvidencePurpose::Dimensions,
                    requested_bytes,
                },
            }) if requested_bytes > 0
        ));
    }
}
