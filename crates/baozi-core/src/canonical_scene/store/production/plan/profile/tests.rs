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
