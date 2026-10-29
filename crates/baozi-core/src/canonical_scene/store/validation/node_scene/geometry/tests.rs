use std::convert::Infallible;

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CacheRow {
    semantic_row: usize,
    entry: GeometryValidationCacheEntry,
}

fn cache_rows(keys: impl IntoIterator<Item = Option<usize>>) -> Vec<CacheRow> {
    keys.into_iter()
        .enumerate()
        .map(|(row, key)| CacheRow {
            semantic_row: row,
            entry: GeometryValidationCacheEntry::new(key, row).expect("test cache identity"),
        })
        .collect()
}

fn sort_and_count(rows: &mut [CacheRow]) -> u64 {
    let mut inspections = 0_u64;
    if rows.len() > 1 {
        assert!(
            sort_geometry_validation_cache_entries(
                rows,
                |row| &mut row.entry,
                usize::BITS,
                &mut || -> Result<bool, Infallible> {
                    inspections += 1;
                    Ok(true)
                },
            )
            .expect("infallible work observer")
        );
    }
    inspections
}

#[test]
fn distinct_buffer_cache_work_grows_linearly_at_delivery_plan_sizes() {
    let mut previous_work = None;
    for count in [256_usize, 1_024, 4_096] {
        let mut rows = cache_rows((0..count).rev().map(Some));
        let work = sort_and_count(&mut rows);
        assert_eq!(work, geometry_validation_cache_sort_work(count).unwrap());
        assert_eq!(work, u64::try_from(count).unwrap() * u64::from(usize::BITS));
        assert!(
            rows.windows(2)
                .all(|pair| pair[0].entry.key < pair[1].entry.key)
        );
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(row.semantic_row, index, "semantic rows must never move");
            assert_eq!(row.entry.key, index + 1);
            assert_eq!(row.entry.row, count - index - 1);
        }
        if let Some(previous) = previous_work {
            assert_eq!(
                work,
                previous * 4,
                "actual inspections must grow with references"
            );
        }
        previous_work = Some(work);
    }
}

#[test]
fn cache_groups_full_slot_indices_and_keeps_absence_separate() {
    let high = 1_usize << (usize::BITS - 1);
    let keys = [
        Some(high + 7),
        None,
        Some(7),
        Some(high + 7),
        Some(7),
        Some(usize::MAX - 1),
    ];
    let mut rows = cache_rows(keys);
    sort_and_count(&mut rows);

    let sorted: Vec<_> = rows.iter().map(|row| row.entry.key).collect();
    assert_eq!(sorted, [0, 8, 8, high + 8, high + 8, usize::MAX]);
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row.semantic_row, index);
        assert_eq!(row.entry.key, keys[row.entry.row].map_or(0, |key| key + 1));
    }
    assert!(GeometryValidationCacheEntry::new(Some(usize::MAX), 0).is_none());
}

#[test]
fn repeated_slot_identities_form_one_contiguous_summary_group() {
    let mut rows = cache_rows((0..4_096).map(|index| Some(index % 3)));
    let work = sort_and_count(&mut rows);
    let groups: Vec<_> = rows
        .chunk_by(|left, right| left.entry.key == right.entry.key)
        .map(|group| (group[0].entry.key, group.len()))
        .collect();
    assert_eq!(groups, [(1, 1_366), (2, 1_365), (3, 1_365)]);
    assert_eq!(work, 4_096 * u64::from(usize::BITS));
}

#[test]
fn cache_work_stops_before_the_first_unadmitted_inspection() {
    let high = 1_usize << (usize::BITS - 1);
    let mut rows =
        cache_rows((0..1_024).map(|index| Some(if index % 2 == 0 { high } else { index })));
    let original = rows.clone();
    let mut attempts = 0;
    let mut completed = 0;
    let accepted = sort_geometry_validation_cache_entries(
        &mut rows,
        |row| &mut row.entry,
        usize::BITS,
        &mut || -> Result<bool, Infallible> {
            attempts += 1;
            if completed == 256 {
                return Ok(false);
            }
            completed += 1;
            Ok(true)
        },
    )
    .expect("infallible work observer");
    assert!(!accepted);
    assert_eq!(attempts, 257);
    assert_eq!(completed, 256);
    assert_eq!(&rows[256..], &original[256..]);
    assert!(
        rows.iter()
            .enumerate()
            .all(|(index, row)| row.semantic_row == index)
    );

    let mut untouched = original.clone();
    let interrupted = sort_geometry_validation_cache_entries(
        &mut untouched,
        |row| &mut row.entry,
        usize::BITS,
        &mut || Err("cancelled"),
    );
    assert_eq!(interrupted, Err("cancelled"));
    assert_eq!(untouched, original);
}

#[test]
fn exact_geometry_work_includes_cache_and_name_row_traversals() {
    for (attributes, primitives, logical_meshes) in
        [(0, 0, 0), (1, 1, 1), (256, 256, 1), (1_024, 4_096, 8)]
    {
        let counts = GeometryCanonicalCounts::new(logical_meshes, primitives, attributes, 0);
        let requirements = GeometryValidationScratchRequirements::from_counts(counts).unwrap();
        let expected = 3 * u64::from(attributes)
            + 3 * u64::from(primitives)
            + u64::from(logical_meshes)
            + geometry_validation_cache_sort_work(usize::try_from(attributes).unwrap()).unwrap()
            + 2 * geometry_validation_cache_sort_work(usize::try_from(primitives).unwrap())
                .unwrap();
        assert_eq!(
            requirements.semantic_projection_work_units(),
            Some(expected)
        );
    }
    assert_eq!(geometry_validation_cache_sort_work(0), Some(0));
    assert_eq!(geometry_validation_cache_sort_work(1), Some(0));
}
