use super::*;

fn metered_counts(segment_count: u32) -> ProfileMeteredCounts {
    ProfileMeteredCounts {
        segment_count,
        resource_count: 1,
        source_count: 1,
        provenance_count: 1,
        operation_count: 0,
        evidence_count: 0,
        parent_count: 0,
        wide_count: 0,
        lineage_text_bytes: 1,
        lineage_digest_bytes: 0,
        owner_count: 2,
        attachment_count: 2,
        metadata_entry_count: 0,
        metadata_text_bytes: 0,
        metadata_payload_bytes: 0,
        metadata_bool_count: 0,
        metadata_i64_count: 0,
        metadata_u64_count: 0,
        metadata_f64_count: 0,
        metadata_f32_count: 0,
        canonical_block_count: 8,
        payload_leaf_count: 1,
        shareable_leaf_count: 4,
    }
}

fn poll_interval() -> usize {
    usize::try_from(SCENE_VALIDATION_POLL_INTERVAL).expect("poll interval must fit usize")
}

#[test]
fn byte_comparison_observes_each_bounded_chunk() {
    let len = poll_interval()
        .checked_mul(2)
        .and_then(|len| len.checked_add(1))
        .expect("test payload length");
    let value = vec![7_u8; len];
    let expected = value.clone();
    let mut observations = Vec::new();

    let result = observe_and_match_bytes(&value, Some(&expected), &mut |amount| {
        observations.push(amount);
        (observations.len() < 2).then_some(())
    });

    assert!(result.is_none());
    assert_eq!(
        observations,
        vec![
            u64::try_from(poll_interval()).expect("poll interval work"),
            u64::try_from(poll_interval()).expect("poll interval work"),
        ]
    );
}

#[test]
fn scalar_validation_polls_before_scanning_each_bounded_chunk() {
    let len = poll_interval()
        .checked_mul(2)
        .and_then(|len| len.checked_add(1))
        .expect("test scalar length");
    let values = vec![11_i64; len];
    let mut cursor = 0;
    let mut observations = Vec::new();
    let mut validated = 0;

    let result = consume_validated_scalar_range(
        &values,
        &mut cursor,
        0,
        u32::try_from(values.len()).expect("test scalar count"),
        &mut |amount| {
            observations.push(amount);
            (observations.len() < 2).then_some(())
        },
        |_index, value| {
            validated += 1;
            *value == 11
        },
    );

    assert!(result.is_none());
    assert_eq!(cursor, 0);
    assert_eq!(validated, poll_interval());
    assert_eq!(
        observations,
        vec![
            u64::try_from(poll_interval()).expect("poll interval work"),
            u64::try_from(poll_interval()).expect("poll interval work"),
        ]
    );
}

#[test]
fn each_profile_segment_is_reserved_read_and_initialized_exactly_once() {
    let one = metered_counts(1);
    let two = metered_counts(2);

    assert_eq!(
        two.scratch_bytes()
            .and_then(|two| one.scratch_bytes().map(|one| two - one)),
        Some(
            u64::try_from(std::mem::size_of::<ProfileSegmentValidationScratch<'static>>())
                .expect("segment scratch size must fit u64")
        )
    );
    assert_eq!(
        two.member_read_work()
            .and_then(|two| one.member_read_work().map(|one| two - one)),
        Some(u64::try_from(PROFILE_SEGMENT_FIELD_COUNT).expect("segment fields must fit u64"))
    );
    assert_eq!(
        two.scratch_initialization_work()
            .and_then(|two| one.scratch_initialization_work().map(|one| two - one)),
        Some(1)
    );
}
