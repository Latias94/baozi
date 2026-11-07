use super::*;
use crate::concrete_store::StoreSemanticCatalogId;
use crate::ledger::finite::{FiniteLedger, FiniteLedgerLimits};
use crate::{DimensionPolicy, OperationOwner};
use std::num::{NonZeroU32, NonZeroU64};

struct Owner;
impl OperationOwner for Owner {}

fn catalog() -> StoreSemanticCatalogId {
    StoreSemanticCatalogId::new(*b"finite-facts-v1!", NonZeroU32::MIN)
}

#[test]
fn finite_facts_keep_original_domain_queries_and_nonzero_rows_after_producer_ends() {
    let catalog = catalog();
    let domain = StoreKeyDomain::try_new(catalog).unwrap();
    let dimensions = [6, 1, 5, 2, 4, 3].map(|local| {
        domain
            .try_admit_dimension(StoreDimensionSemanticId::new(
                catalog,
                NonZeroU64::new(local).unwrap(),
            ))
            .unwrap()
    });
    let phases = [3, 1, 2].map(|local| {
        domain
            .try_admit_phase(StorePhaseSemanticId::new(
                catalog,
                NonZeroU64::new(local).unwrap(),
            ))
            .unwrap()
    });
    let absent_dimension = domain
        .try_admit_dimension(StoreDimensionSemanticId::new(
            catalog,
            NonZeroU64::new(7).unwrap(),
        ))
        .unwrap();
    let absent_phase = domain
        .try_admit_phase(StorePhaseSemanticId::new(
            catalog,
            NonZeroU64::new(4).unwrap(),
        ))
        .unwrap();
    let facts = {
        let limits = FiniteLedgerLimits::try_new(
            u64::MAX,
            u64::MAX,
            dimensions.map(|key| (key, DimensionPolicy::counter(100, 100, 0))),
        )
        .unwrap();
        let mut ledger = FiniteLedger::<Owner, _, _>::try_new(limits, phases).unwrap();
        ledger
            .view()
            .charge_observed_batch(&[(dimensions[0], 9)])
            .unwrap();
        ledger.view().try_complete_phase(phases[1]).unwrap();
        let (snapshot, cause) = ledger.take_terminal_snapshot(0);
        assert_eq!(cause, None);
        FiniteStoreFacts::from_snapshot(snapshot, StoreTerminalCause::Success, domain)
    };
    let header = facts.header();
    assert_eq!(header.terminal_cause(), StoreTerminalCause::Success);
    assert_eq!(header.dimension_count(), 6);
    assert_eq!(header.phase_count(), 3);
    assert_eq!(header.last_completed_phase(), Some(phases[1].semantic_id()));
    assert_eq!(facts.covered_administrative_bytes(), Some(0));
    let work = facts.dimension(dimensions[0]).unwrap();
    assert_eq!((work.attempted(), work.consumed()), (9, 9));
    assert_eq!(
        facts.dimension_by_semantic(dimensions[0].semantic_id()),
        Some(work)
    );
    assert_eq!(facts.dimension_at(5), Some(work));
    assert_eq!(facts.dimension_at(0).unwrap().dimension().local().get(), 1);
    assert_eq!(facts.dimension_at(6), None);
    assert_eq!(facts.dimension_at(usize::MAX), None);
    assert_eq!(facts.phase_at(0).unwrap().phase().local().get(), 1);
    assert_eq!(facts.phase_at(2).unwrap().phase().local().get(), 3);
    assert_eq!(facts.phase_at(3), None);
    assert_eq!(facts.phase_at(usize::MAX), None);
    assert_eq!(
        facts.phase_by_semantic(phases[0].semantic_id()),
        facts.phase(phases[0]).ok()
    );
    assert_eq!(
        facts.dimension(absent_dimension),
        Err(StoreRuntimeError::Ledger(LedgerCause::UnknownDimension {
            dimension: absent_dimension
        }))
    );
    assert_eq!(
        facts.phase(absent_phase),
        Err(StoreRuntimeError::Ledger(LedgerCause::UnknownPhase))
    );
    assert_eq!(
        facts.dimension_by_semantic(absent_dimension.semantic_id()),
        None
    );
    assert_eq!(facts.phase_by_semantic(absent_phase.semantic_id()), None);

    let foreign = StoreKeyDomain::try_new(catalog).unwrap();
    let foreign_dimension = foreign
        .try_admit_dimension(dimensions[0].semantic_id())
        .unwrap();
    let foreign_phase = foreign.try_admit_phase(phases[0].semantic_id()).unwrap();
    assert_eq!(
        facts.dimension(foreign_dimension),
        Err(StoreRuntimeError::ForeignDimensionDomain {
            dimension: foreign_dimension
        })
    );
    assert_eq!(
        facts.phase(foreign_phase),
        Err(StoreRuntimeError::ForeignPhaseDomain {
            phase: foreign_phase
        })
    );
    let foreign_catalog = StoreSemanticCatalogId::new(*b"foreign-facts!!!", NonZeroU32::MIN);
    assert_eq!(
        facts.dimension_by_semantic(StoreDimensionSemanticId::new(
            foreign_catalog,
            NonZeroU64::MIN
        )),
        None
    );
    assert_eq!(
        facts.phase_by_semantic(StorePhaseSemanticId::new(foreign_catalog, NonZeroU64::MIN)),
        None
    );
    assert_eq!(
        (
            facts.dimension(dimensions[0]).unwrap().attempted(),
            facts.dimension(dimensions[0]).unwrap().consumed()
        ),
        (9, 9)
    );
}
