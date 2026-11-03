use super::*;

#[test]
fn inline_semantic_totals_do_not_add_another_payload_charge() {
    let mut metrics = ProfileMetrics {
        owner_attachments: 2,
        evidence_count: 1,
        lineage_digest_bytes: 32,
        ..ProfileMetrics::default()
    };
    let before = retention_shape_from_metrics(1, 1, 2, 1, 2, 0, &metrics)
        .expect("the descriptor shape must be bounded");
    metrics.wide_count = 2;
    metrics.lineage_digest_bytes = 96;
    let after = retention_shape_from_metrics(1, 1, 2, 1, 2, 0, &metrics)
        .expect("inline source and evidence values keep the same descriptor shape");
    assert_eq!(after.canonical_bytes(), before.canonical_bytes());
    assert_eq!(after.wide_count(), 2);
    assert_eq!(after.lineage_digest_bytes(), 96);
}

#[test]
fn metadata_payload_growth_is_counted_with_its_typed_entry() {
    let mut metrics = ProfileMetrics {
        owner_attachments: 2,
        lineage_digest_bytes: 32,
        ..ProfileMetrics::default()
    };
    let before = retention_shape_from_metrics(0, 0, 1, 1, 2, 0, &metrics)
        .expect("the base profile must be bounded");
    metrics.metadata_text_bytes = 19;
    metrics.metadata_f32_count = 3;
    let after = retention_shape_from_metrics(0, 0, 1, 1, 2, 1, &metrics)
        .expect("one named Vec3 metadata entry must be bounded");
    assert_eq!(
        after.canonical_bytes() - before.canonical_bytes(),
        19 + u64::try_from(size_of::<[f32; 3]>() + size_of::<StoredMetadataEntry>())
            .expect("the metadata fixture byte count fits u64")
    );
}

#[test]
fn typed_table_bytes_reject_multiplication_overflow() {
    let row_bytes = u64::try_from(size_of::<StoredSource>()).expect("row size fits u64");
    let maximum_count = u64::MAX / row_bytes;
    assert_eq!(
        typed_table_bytes::<StoredSource>(maximum_count, "source table").expect("last valid count"),
        maximum_count * row_bytes
    );
    assert!(matches!(
        typed_table_bytes::<StoredSource>(maximum_count + 1, "source table"),
        Err(ProfileAuthoringSemanticError::AccountingOverflow {
            dimension: "source table",
        })
    ));
}

#[test]
fn semantic_counts_still_obey_the_canonical_u32_domain() {
    let metrics = ProfileMetrics {
        lineage_digest_bytes: u64::from(u32::MAX) + 1,
        ..ProfileMetrics::default()
    };
    assert!(matches!(
        retention_shape_from_metrics(0, 0, 1, 1, 2, 0, &metrics),
        Err(ProfileAuthoringSemanticError::CountDomainTooLarge {
            collection: "profile lineage digest bytes",
        })
    ));
}

#[test]
fn zero_payload_cardinality_never_enters_control_or_validates_the_existing_metric() {
    struct NoControlCalls;

    impl CanonicalAuthoringControl for NoControlCalls {
        type Error = std::convert::Infallible;
        type Reservation = ();

        fn poll(&mut self) -> Result<(), Self::Error> {
            panic!("zero cardinality must not poll");
        }

        fn run_unit<R>(
            &mut self,
            _work: SceneValidationWork,
            _action: impl FnMut() -> R,
        ) -> Result<R, Self::Error> {
            panic!("zero cardinality must not admit an action");
        }

        fn reserve_scratch(&mut self, _bytes: u64) -> Result<(), Self::Error> {
            panic!("zero cardinality must not reserve scratch");
        }

        fn validate_scratch(&mut self, _reservation: &()) -> Result<(), Self::Error> {
            panic!("zero cardinality must not validate scratch");
        }

        fn resize_scratch(
            &mut self,
            _reservation: &mut (),
            _bytes: u64,
        ) -> Result<(), Self::Error> {
            panic!("zero cardinality must not resize scratch");
        }
    }

    let mut metrics = ProfileMetrics {
        payload_cells: 3,
        ..ProfileMetrics::default()
    };
    metrics
        .add_payload_cardinality(0, 2)
        .expect("zero must preserve an already excessive metric");
    let mut control = NoControlCalls;
    let limits = SceneValidationLimits::default()
        .with_max_payload_cells(2)
        .with_max_work(0);
    let mut context = ProfileValidationContext::new(limits, &mut control);
    context.work = u64::MAX;
    context.work_since_poll = 255;
    metrics
        .add_payload(0, &mut context)
        .expect("zero must skip even local work overflow and a pending poll");
    assert_eq!(metrics.payload_cells, 3);
    assert_eq!(context.work, u64::MAX);
    assert_eq!(context.work_since_poll, 255);
}

#[test]
fn payload_cardinality_accepts_the_entire_u64_domain_without_visiting_cells() {
    let mut metrics = ProfileMetrics::default();
    metrics
        .add_payload_cardinality(u64::MAX, u64::MAX)
        .expect("one bounded arithmetic update must accept the exact maximum");
    assert_eq!(metrics.payload_cells, u64::MAX);
}

#[test]
fn payload_cardinality_limit_is_inclusive() {
    let mut metrics = ProfileMetrics {
        payload_cells: 3,
        ..ProfileMetrics::default()
    };
    metrics
        .add_payload_cardinality(4, 7)
        .expect("the exact logical cell limit must be inclusive");
    assert_eq!(metrics.payload_cells, 7);
}

#[test]
fn payload_cardinality_rejection_keeps_the_accepted_prefix() {
    let mut metrics = ProfileMetrics {
        payload_cells: 3,
        ..ProfileMetrics::default()
    };
    assert!(matches!(
        metrics.add_payload_cardinality(9, 7),
        Err(ProfileAuthoringSemanticError::LimitExceeded {
            limit: ProfileAuthoringLimit::PayloadCells,
            actual: 8,
            maximum: 7,
        })
    ));
    assert_eq!(metrics.payload_cells, 7);
}

#[test]
fn payload_cardinality_limit_precedes_a_later_total_overflow() {
    let mut metrics = ProfileMetrics {
        payload_cells: u64::MAX - 3,
        ..ProfileMetrics::default()
    };
    assert!(matches!(
        metrics.add_payload_cardinality(10, u64::MAX - 1),
        Err(ProfileAuthoringSemanticError::LimitExceeded {
            limit: ProfileAuthoringLimit::PayloadCells,
            actual: u64::MAX,
            maximum,
        }) if maximum == u64::MAX - 1
    ));
    assert_eq!(metrics.payload_cells, u64::MAX - 1);
}

#[test]
fn payload_cardinality_overflow_keeps_the_representable_prefix() {
    let mut metrics = ProfileMetrics {
        payload_cells: u64::MAX - 2,
        ..ProfileMetrics::default()
    };
    assert!(matches!(
        metrics.add_payload_cardinality(3, u64::MAX),
        Err(ProfileAuthoringSemanticError::AccountingOverflow {
            dimension: "profile payload cells",
        })
    ));
    assert_eq!(metrics.payload_cells, u64::MAX);
    assert!(matches!(
        metrics.add_payload_cardinality(1, u64::MAX - 1),
        Err(ProfileAuthoringSemanticError::AccountingOverflow {
            dimension: "profile payload cells",
        })
    ));
    assert_eq!(metrics.payload_cells, u64::MAX);
}

#[test]
fn payload_cardinality_rejects_an_excessive_first_successor_without_mutation() {
    let mut metrics = ProfileMetrics {
        payload_cells: 9,
        ..ProfileMetrics::default()
    };
    assert!(matches!(
        metrics.add_payload_cardinality(1, 7),
        Err(ProfileAuthoringSemanticError::LimitExceeded {
            limit: ProfileAuthoringLimit::PayloadCells,
            actual: 10,
            maximum: 7,
        })
    ));
    assert_eq!(metrics.payload_cells, 9);
}
