use super::*;

fn counts() -> ProfileMeteredCounts {
    ProfileMeteredCounts {
        segment_count: 1,
        resource_count: 1,
        source_count: 1,
        provenance_count: 1,
        operation_count: 0,
        evidence_count: 0,
        parent_count: 0,
        wide_count: 1,
        lineage_text_bytes: 10,
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
    }
}

#[test]
fn profile_work_separates_depth_initialization_from_borrowed_validation() {
    let counts = counts();
    assert_eq!(counts.scratch_bytes(), Some(4));
    assert_eq!(counts.scratch_initialization_work(), Some(1));
    assert_eq!(counts.validation_work(), Some(25));
    assert_eq!(
        ProfileMeteredScratch::required_provenance_depths(counts.provenance_count),
        Some(1),
    );
}

#[test]
fn each_owner_has_separate_header_and_depth_cache_validation_actions() {
    let one = counts();
    let two = ProfileMeteredCounts {
        owner_count: one.owner_count + 1,
        ..one
    };
    assert_eq!(
        two.validation_work().unwrap() - one.validation_work().unwrap(),
        2,
    );
    assert_eq!(one.scratch_bytes(), two.scratch_bytes());
    assert_eq!(
        one.scratch_initialization_work(),
        two.scratch_initialization_work(),
    );
}

#[test]
fn only_provenance_growth_increases_profile_scratch() {
    let small = counts();
    let large = ProfileMeteredCounts {
        segment_count: 17,
        resource_count: 1_048_576,
        source_count: 1_048_576,
        operation_count: 1_048_576,
        evidence_count: 1_048_576,
        parent_count: 1_048_576,
        wide_count: 1_048_576,
        lineage_text_bytes: 1_048_576,
        lineage_digest_bytes: 1_048_576,
        owner_count: 1_048_576,
        attachment_count: 1_048_576,
        metadata_entry_count: 1_048_576,
        metadata_text_bytes: 1_048_576,
        metadata_payload_bytes: 1_048_576,
        metadata_bool_count: 1_048_576,
        metadata_i64_count: 1_048_576,
        metadata_u64_count: 1_048_576,
        metadata_f64_count: 1_048_576,
        metadata_f32_count: 1_048_576,
        ..small
    };
    assert_eq!(small.scratch_bytes(), large.scratch_bytes());
    assert_eq!(
        small.scratch_initialization_work(),
        large.scratch_initialization_work(),
    );
    let deeper = ProfileMeteredCounts {
        provenance_count: 2,
        ..large
    };
    assert_eq!(deeper.scratch_bytes(), Some(8));
    assert_eq!(deeper.scratch_initialization_work(), Some(2));
    assert_eq!(
        deeper.validation_work().unwrap() - large.validation_work().unwrap(),
        2,
    );
}

#[test]
fn typed_inline_totals_do_not_schedule_phantom_scalar_or_digest_array_walks() {
    let small = counts();
    let large = ProfileMeteredCounts {
        wide_count: u32::MAX,
        lineage_digest_bytes: u32::MAX,
        ..small
    };
    assert_eq!(small.validation_work(), large.validation_work());
}
