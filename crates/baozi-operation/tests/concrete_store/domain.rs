use super::*;

#[test]
fn store_semantic_identity_bytes_are_exact_brand_neutral_big_endian_goldens() {
    let namespace = [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
        0xff,
    ];
    let catalog = StoreSemanticCatalogId::new(
        namespace,
        NonZeroU32::new(0x0102_0304).expect("golden revision is nonzero"),
    );
    let local = NonZeroU64::new(0x0102_0304_0506_0708).expect("golden local id is nonzero");
    let phase = StorePhaseSemanticId::new(catalog, local);
    let dimension = StoreDimensionSemanticId::new(catalog, local);

    assert_eq!(
        catalog.to_canonical_bytes(),
        [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff, 0x01, 0x02, 0x03, 0x04,
        ]
    );
    assert_eq!(
        phase.to_canonical_bytes(),
        [
            0x01, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc,
            0xdd, 0xee, 0xff, 0x01, 0x02, 0x03, 0x04, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
            0x08,
        ]
    );
    assert_eq!(
        dimension.to_canonical_bytes(),
        [
            0x02, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc,
            0xdd, 0xee, 0xff, 0x01, 0x02, 0x03, 0x04, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
            0x08,
        ]
    );

    let first_domain = StoreKeyDomain::try_new(catalog).expect("first runtime authority");
    let second_domain = StoreKeyDomain::try_new(catalog).expect("second runtime authority");
    let first_key = first_domain
        .try_admit_phase(phase)
        .expect("first host admission");
    let second_key = second_domain
        .try_admit_phase(phase)
        .expect("second host admission");
    assert_ne!(first_key, second_key);
    assert_eq!(
        first_key.semantic_id().to_canonical_bytes(),
        second_key.semantic_id().to_canonical_bytes(),
        "the private runtime brand is never part of canonical identity bytes"
    );

    let mut changed_namespace = namespace;
    changed_namespace[0] = 0x01;
    let namespace_variant = StoreSemanticCatalogId::new(
        changed_namespace,
        NonZeroU32::new(0x0102_0304).expect("variant revision is nonzero"),
    );
    let revision_variant = StoreSemanticCatalogId::new(
        namespace,
        NonZeroU32::new(0x0102_0305).expect("variant revision is nonzero"),
    );
    let local_variant = StorePhaseSemanticId::new(
        catalog,
        NonZeroU64::new(0x0102_0304_0506_0709).expect("variant local id is nonzero"),
    );
    assert_ne!(
        catalog.to_canonical_bytes(),
        namespace_variant.to_canonical_bytes()
    );
    assert_ne!(
        catalog.to_canonical_bytes(),
        revision_variant.to_canonical_bytes()
    );
    assert_ne!(
        phase.to_canonical_bytes(),
        local_variant.to_canonical_bytes()
    );

    let foreign_catalog = StoreSemanticCatalogId::new(
        *b"foreign-catalog!",
        NonZeroU32::new(1).expect("foreign revision is nonzero"),
    );
    assert!(
        first_domain
            .try_admit_phase(StorePhaseSemanticId::new(
                foreign_catalog,
                NonZeroU64::new(1).expect("foreign phase id is nonzero"),
            ))
            .is_none(),
        "declaring a semantic ID does not grant host admission"
    );
    assert!(
        first_domain
            .try_admit_phase(StorePhaseSemanticId::new(
                revision_variant,
                NonZeroU64::new(1).expect("revision-variant phase id is nonzero"),
            ))
            .is_none(),
        "matching a namespace without its revision does not grant phase admission"
    );
    assert!(
        first_domain
            .try_admit_dimension(StoreDimensionSemanticId::new(
                revision_variant,
                NonZeroU64::new(1).expect("revision-variant dimension id is nonzero"),
            ))
            .is_none(),
        "matching a namespace without its revision does not grant dimension admission"
    );
}

#[test]
fn concrete_store_keys_are_fixed_inert_non_zero_values() {
    let TestVocabulary { domain, keys } = test_vocabulary();
    let foreign_domain = StoreKeyDomain::try_new(test_catalog()).unwrap();
    let foreign_phase = foreign_domain.try_admit_phase(phase_semantic(1)).unwrap();
    let foreign_dimension = foreign_domain
        .try_admit_dimension(dimension_semantic(1))
        .unwrap();

    assert_eq!(
        std::mem::size_of::<StorePhaseKey>(),
        std::mem::size_of::<StoreDimensionKey>()
    );
    assert_eq!(
        std::mem::size_of::<Option<StorePhaseKey>>(),
        std::mem::size_of::<StorePhaseKey>()
    );
    assert_eq!(
        std::mem::size_of::<Option<StoreDimensionKey>>(),
        std::mem::size_of::<StoreDimensionKey>()
    );
    assert!(!std::mem::needs_drop::<StorePhaseKey>());
    assert!(!std::mem::needs_drop::<StoreDimensionKey>());
    assert!(!std::mem::needs_drop::<StoreKeyDomain>());
    assert_eq!(keys.phase.semantic_id(), phase_semantic(1));
    assert_eq!(keys.dimension.semantic_id(), dimension_semantic(1));
    assert_ne!(keys.phase, foreign_phase);
    assert_ne!(keys.dimension, foreign_dimension);
    assert_eq!(format!("{domain:?}"), format!("{foreign_domain:?}"));
    assert_eq!(format!("{:?}", keys.phase), format!("{foreign_phase:?}"));
    assert_eq!(
        format!("{:?}", keys.dimension),
        format!("{foreign_dimension:?}")
    );
}

#[test]
fn concrete_store_key_hashing_exposes_only_the_structured_semantic_identity() {
    let TestVocabulary { keys, .. } = test_vocabulary();
    assert_eq!(
        hash_transcript(&keys.phase),
        hash_transcript(&foreign_phase())
    );
    assert_eq!(
        hash_transcript(&keys.phase),
        hash_transcript(&keys.phase.semantic_id())
    );
    assert_ne!(
        hash_transcript(&keys.phase),
        hash_transcript(&keys.other_phase)
    );
    assert_eq!(
        hash_transcript(&keys.dimension),
        hash_transcript(&foreign_dimension())
    );
    assert_eq!(
        hash_transcript(&keys.dimension),
        hash_transcript(&keys.dimension.semantic_id())
    );
    assert_ne!(
        hash_transcript(&keys.dimension),
        hash_transcript(&keys.other_dimension)
    );
}

#[test]
fn store_ledger_plan_rejects_incomplete_or_non_canonical_phase_catalogs() {
    let limits = |keys: TestKeys| {
        LedgerLimits::new(
            16,
            [(
                keys.dimension,
                DimensionPolicy::reservable_bytes(16, 16, 16, 16),
            )],
        )
        .unwrap()
    };

    let TestVocabulary { domain, keys } = test_vocabulary();
    assert!(matches!(
        StoreLedgerPlan::try_new(domain, limits(keys), &[]),
        Err(StoreLedgerPlanError::EmptyPhaseCatalog)
    ));
    let TestVocabulary { domain, keys } = test_vocabulary();
    assert!(matches!(
        StoreLedgerPlan::try_new(domain, limits(keys), &[keys.phase, keys.phase]),
        Err(StoreLedgerPlanError::DuplicatePhase { phase }) if phase == keys.phase
    ));
    let TestVocabulary { domain, keys } = test_vocabulary();
    assert!(matches!(
        StoreLedgerPlan::try_new(domain, limits(keys), &[keys.other_phase, keys.phase]),
        Err(StoreLedgerPlanError::PhaseOutOfOrder { previous, phase })
            if previous == keys.other_phase && phase == keys.phase
    ));
    let TestVocabulary { domain, keys } = test_vocabulary();
    let foreign_phase = foreign_phase();
    assert!(matches!(
        StoreLedgerPlan::try_new(domain, limits(keys), &[foreign_phase]),
        Err(StoreLedgerPlanError::ForeignPhaseDomain { phase }) if phase == foreign_phase
    ));
    let TestVocabulary { domain, keys } = test_vocabulary();
    let foreign_dimension = foreign_dimension();
    let foreign_limits = LedgerLimits::new(
        16,
        [(
            foreign_dimension,
            DimensionPolicy::reservable_bytes(16, 16, 16, 16),
        )],
    )
    .unwrap();
    assert!(matches!(
        StoreLedgerPlan::try_new(domain, foreign_limits, &[keys.phase]),
        Err(StoreLedgerPlanError::ForeignDimensionDomain { dimension })
            if dimension == foreign_dimension
    ));

    let TestVocabulary { domain, keys } = test_vocabulary();
    let plan =
        StoreLedgerPlan::try_new(domain, limits(keys), &[keys.phase, keys.other_phase]).unwrap();
    assert_eq!(plan.dimension_count(), 1);
    assert_eq!(plan.phase_count(), 2);
}

#[test]
fn retained_store_start_rejects_phase_outside_frozen_catalog_without_retained_side_effects() {
    let factory = factory(1);
    let account = account(&factory);
    let before = account.snapshot();
    let TestVocabulary { domain, keys } = test_vocabulary();
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(u64::MAX);
    let limits = LedgerLimits::new(u64::MAX, [(keys.dimension, policy)]).unwrap();
    let ledger_plan = StoreLedgerPlan::try_new(domain, limits, &[keys.phase]).unwrap();
    let start = RetainedStoreStartPlan::new(
        RetainedStoreWriterPlan::new(
            keys.other_phase,
            keys.dimension,
            keys.dimension,
            RetainedStoreBatchLimits::new(8, 4).unwrap(),
        ),
        RetainedStoreSharePlan::new(keys.dimension, keys.dimension),
        StoreFallbackEnvelope::new(91),
        keys.dimension,
        1,
        keys.dimension,
        1,
    );

    let rejected =
        TestOperation::try_start(OperationControl::default(), ledger_plan, &account, start);

    let cause = match rejected {
        Err(cause) => cause,
        Ok(operation) => {
            drop(operation);
            panic!("phase outside the frozen catalog must be rejected");
        }
    };
    assert!(
        matches!(
            cause,
            RetainedStoreStartError::Runtime(StoreRuntimeError::Ledger(LedgerCause::UnknownPhase))
        ),
        "unexpected frozen phase rejection: {cause:?}"
    );
    assert_eq!(account.snapshot(), before);
}

#[test]
fn retained_store_start_preflights_every_dimension_key_without_side_effects() {
    let factory = factory(1);
    let account = account(&factory);
    for invalid_position in 0..6 {
        let TestVocabulary { domain, keys } = test_vocabulary();
        let mut dimensions = [keys.dimension; 6];
        dimensions[invalid_position] = keys.other_dimension;
        let before = account.snapshot();
        let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
            .with_published_outcome_evidence_limit(u64::MAX);
        let ledger_limits = LedgerLimits::new(u64::MAX, [(keys.dimension, policy)]).unwrap();
        let ledger_plan = StoreLedgerPlan::try_new(domain, ledger_limits, &[keys.phase]).unwrap();
        let start = RetainedStoreStartPlan::new(
            RetainedStoreWriterPlan::new(
                keys.phase,
                dimensions[0],
                dimensions[1],
                RetainedStoreBatchLimits::new(8, 4).unwrap(),
            ),
            RetainedStoreSharePlan::new(dimensions[2], dimensions[3]),
            StoreFallbackEnvelope::new(91),
            dimensions[4],
            1,
            dimensions[5],
            1,
        );

        let rejected =
            TestOperation::try_start(OperationControl::default(), ledger_plan, &account, start);

        assert!(matches!(
            rejected,
            Err(RetainedStoreStartError::Runtime(StoreRuntimeError::Ledger(
                LedgerCause::UnknownDimension { dimension }
            ))) if dimension == keys.other_dimension
        ));
        assert_eq!(account.snapshot(), before);
    }
}

#[test]
fn retained_store_start_rejects_foreign_phase_and_dimension_domains_without_side_effects() {
    let factory = factory(1);
    let account = account(&factory);
    for reject_foreign_phase in [true, false] {
        let TestVocabulary { domain, keys } = test_vocabulary();
        let foreign_phase = foreign_phase();
        let foreign_dimension = foreign_dimension();
        let (phase, dimension, expected) = if reject_foreign_phase {
            (
                foreign_phase,
                keys.dimension,
                StoreRuntimeError::ForeignPhaseDomain {
                    phase: foreign_phase,
                },
            )
        } else {
            (
                keys.phase,
                foreign_dimension,
                StoreRuntimeError::ForeignDimensionDomain {
                    dimension: foreign_dimension,
                },
            )
        };
        let before = account.snapshot();
        let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
            .with_published_outcome_evidence_limit(u64::MAX);
        let limits = LedgerLimits::new(u64::MAX, [(keys.dimension, policy)]).unwrap();
        let ledger_plan = StoreLedgerPlan::try_new(domain, limits, &[keys.phase]).unwrap();
        let start = RetainedStoreStartPlan::new(
            RetainedStoreWriterPlan::new(
                phase,
                dimension,
                keys.dimension,
                RetainedStoreBatchLimits::new(8, 4).unwrap(),
            ),
            RetainedStoreSharePlan::new(keys.dimension, keys.dimension),
            StoreFallbackEnvelope::new(91),
            keys.dimension,
            1,
            keys.dimension,
            1,
        );

        let rejected =
            TestOperation::try_start(OperationControl::default(), ledger_plan, &account, start);

        assert!(matches!(
            rejected,
            Err(RetainedStoreStartError::Runtime(cause)) if cause == expected
        ));
        assert_eq!(account.snapshot(), before);
    }
}

#[test]
fn publication_key_preflight_rejects_unknown_phase_and_dimensions_without_side_effects() {
    let factory = factory(1);
    let account = account(&factory);
    let limits = RetainedStoreBatchLimits::new(2, 2).unwrap();

    let phase_cause = reject_publication_plan_without_side_effects(&account, |keys| {
        publication_plan_with_writer(RetainedStoreWriterPlan::new(
            keys.other_phase,
            keys.dimension,
            keys.dimension,
            limits,
        ))
    });
    assert!(matches!(
        phase_cause,
        StoreRuntimeError::Ledger(LedgerCause::UnknownPhase)
    ));

    for unknown_attribution_dimension in [true, false] {
        let dimension_cause = reject_publication_plan_without_side_effects(&account, |keys| {
            let writer = if unknown_attribution_dimension {
                RetainedStoreWriterPlan::new(
                    keys.phase,
                    keys.other_dimension,
                    keys.dimension,
                    limits,
                )
            } else {
                RetainedStoreWriterPlan::new(
                    keys.phase,
                    keys.dimension,
                    keys.other_dimension,
                    limits,
                )
            };
            publication_plan_with_writer(writer)
        });
        assert!(matches!(
            dimension_cause,
            StoreRuntimeError::Ledger(LedgerCause::UnknownDimension { .. })
        ));
    }
}

#[test]
fn publication_key_preflight_rejects_foreign_domains_without_side_effects() {
    let factory = factory(1);
    let account = account(&factory);
    let limits = RetainedStoreBatchLimits::new(2, 2).unwrap();

    let foreign_phase = foreign_phase();
    let phase_cause = reject_publication_plan_without_side_effects(&account, |keys| {
        publication_plan_with_writer(RetainedStoreWriterPlan::new(
            foreign_phase,
            keys.dimension,
            keys.dimension,
            limits,
        ))
    });
    assert!(matches!(
        phase_cause,
        StoreRuntimeError::ForeignPhaseDomain { phase } if phase == foreign_phase
    ));

    let foreign_dimension = foreign_dimension();
    let dimension_cause = reject_publication_plan_without_side_effects(&account, |keys| {
        publication_plan_with_writer(RetainedStoreWriterPlan::new(
            keys.phase,
            foreign_dimension,
            keys.dimension,
            limits,
        ))
    });
    assert!(matches!(
        dimension_cause,
        StoreRuntimeError::ForeignDimensionDomain { dimension }
            if dimension == foreign_dimension
    ));
}
