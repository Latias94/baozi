use super::*;

use std::mem::discriminant;

use StoredMetadataValue as Value;

#[derive(Default)]
struct Control {
    completed: u64,
    string_bytes: u64,
    reject_before: Option<u64>,
}

impl Control {
    fn run<R>(&mut self, string: bool, action: impl FnOnce() -> R) -> Result<R, u64> {
        if self.reject_before == Some(self.completed) {
            return Err(self.completed);
        }
        let result = action();
        self.completed += 1;
        self.string_bytes += u64::from(string);
        Ok(result)
    }
}

impl TypedStorageBuildControl for Control {
    type Error = u64;

    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        self.run(false, action)
    }

    fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        self.run(true, action)
    }
}

fn empty_payloads() -> MetadataPayloads {
    MetadataPayloads {
        text: String::new(),
        bytes: vec![],
        bools: vec![],
        i64s: vec![],
        u64s: vec![],
        f64s: vec![],
        f32s: vec![],
    }
}

fn store_text(pool: &mut String, value: &str) -> TextRange {
    let range = TextRange(TableRange::new(pool.len(), value.len()));
    pool.push_str(value);
    range
}

fn store<T: Copy>(pool: &mut Vec<T>, values: &[T]) -> TableRange<T> {
    let range = TableRange::new(pool.len(), values.len());
    pool.extend_from_slice(values);
    range
}

fn add(
    entries: &mut Vec<StoredMetadataEntry>,
    pools: &mut MetadataPayloads,
    owner: CanonicalProfileOwner,
    key: &str,
    value: impl FnOnce(&mut MetadataPayloads) -> StoredMetadataValue,
) {
    let key = store_text(&mut pools.text, key);
    let value = value(pools);
    entries.push(StoredMetadataEntry { owner, key, value });
}

fn lengths(pools: &MetadataPayloads) -> [usize; 7] {
    [
        pools.text.len(),
        pools.bytes.len(),
        pools.bools.len(),
        pools.i64s.len(),
        pools.u64s.len(),
        pools.f64s.len(),
        pools.f32s.len(),
    ]
}

const DELETED: CanonicalProfileOwner = CanonicalProfileOwner::Attribute(1);
const KEPT: CanonicalProfileOwner = CanonicalProfileOwner::Attribute(2);

fn mixed_fixture() -> (
    Vec<StoredMetadataEntry>,
    MetadataPayloads,
    [usize; 7],
    [usize; 7],
) {
    let mut entries = vec![];
    let mut p = empty_payloads();
    add(
        &mut entries,
        &mut p,
        CanonicalProfileOwner::Scene,
        "test:a",
        |p| Value::String(store_text(&mut p.text, "first α")),
    );
    add(
        &mut entries,
        &mut p,
        CanonicalProfileOwner::Scene,
        "test:b",
        |p| Value::I64(store(&mut p.i64s, &[-1])),
    );
    add(
        &mut entries,
        &mut p,
        CanonicalProfileOwner::Scene,
        "test:c",
        |p| Value::Vec2(store(&mut p.f32s, &[-0.0, 1.0])),
    );
    let start = lengths(&p);
    add(&mut entries, &mut p, DELETED, "test:a", |p| {
        Value::BoolArray(store(&mut p.bools, &[1, 0]))
    });
    add(&mut entries, &mut p, DELETED, "test:b", |p| {
        Value::I64Array(store(&mut p.i64s, &[7, 8]))
    });
    add(&mut entries, &mut p, DELETED, "test:c", |p| {
        Value::U64Array(store(&mut p.u64s, &[91, 92]))
    });
    add(&mut entries, &mut p, DELETED, "test:d", |p| {
        Value::F64Array(store(&mut p.f64s, &[13.0, -0.0]))
    });
    add(&mut entries, &mut p, DELETED, "test:e", |p| {
        Value::String(store_text(&mut p.text, "remove 💥"))
    });
    add(&mut entries, &mut p, DELETED, "test:f", |p| Value::Bytes {
        media_type: store_text(&mut p.text, "application/x-discard"),
        payload: store(&mut p.bytes, &[77, 88]),
    });
    add(&mut entries, &mut p, DELETED, "test:g", |p| {
        Value::Mat4Array(store(&mut p.f32s, &[-2.0; 16]))
    });
    let end = lengths(&p);
    add(&mut entries, &mut p, KEPT, "test:a", |p| {
        Value::Bool(store(&mut p.bools, &[1]))
    });
    add(&mut entries, &mut p, KEPT, "test:b", |p| {
        Value::I64(store(&mut p.i64s, &[i64::MIN]))
    });
    add(&mut entries, &mut p, KEPT, "test:c", |p| {
        Value::U64(store(&mut p.u64s, &[u64::MAX]))
    });
    add(&mut entries, &mut p, KEPT, "test:d", |p| {
        Value::F64(store(&mut p.f64s, &[-0.0]))
    });
    add(&mut entries, &mut p, KEPT, "test:e", |p| {
        Value::String(store_text(&mut p.text, "kept 世界 😀\0"))
    });
    add(&mut entries, &mut p, KEPT, "test:f", |p| Value::Bytes {
        media_type: store_text(&mut p.text, "application/octet-stream"),
        payload: store(&mut p.bytes, &[0, 255, 19, 128]),
    });
    add(&mut entries, &mut p, KEPT, "test:g", |p| {
        Value::Vec2(store(&mut p.f32s, &[-0.0, f32::from_bits(1)]))
    });
    add(&mut entries, &mut p, KEPT, "test:h", |p| {
        Value::Vec3(store(&mut p.f32s, &[1.0, 2.0, 3.0]))
    });
    add(&mut entries, &mut p, KEPT, "test:i", |p| {
        Value::Vec4(store(&mut p.f32s, &[4.0, 5.0, 6.0, 7.0]))
    });
    add(&mut entries, &mut p, KEPT, "test:j", |p| {
        Value::Color(store(&mut p.f32s, &[0.0, 0.25, 0.5, 1.0]))
    });
    add(&mut entries, &mut p, KEPT, "test:k", |p| {
        Value::Mat4(store(&mut p.f32s, &[8.0; 16]))
    });
    add(&mut entries, &mut p, KEPT, "test:l", |p| {
        Value::BoolArray(store(&mut p.bools, &[0, 1, 0]))
    });
    add(&mut entries, &mut p, KEPT, "test:m", |p| {
        Value::I64Array(store(&mut p.i64s, &[0, i64::MAX]))
    });
    add(&mut entries, &mut p, KEPT, "test:n", |p| {
        Value::U64Array(store(&mut p.u64s, &[0, 1, u64::MAX]))
    });
    add(&mut entries, &mut p, KEPT, "test:o", |p| {
        Value::F64Array(store(&mut p.f64s, &[f64::from_bits(1), f64::MAX]))
    });
    add(&mut entries, &mut p, KEPT, "test:p", |p| {
        Value::Vec2Array(store(&mut p.f32s, &[9.0, 10.0, 11.0, 12.0]))
    });
    add(&mut entries, &mut p, KEPT, "test:q", |p| {
        Value::Vec3Array(store(&mut p.f32s, &[13.0, 14.0, 15.0]))
    });
    add(&mut entries, &mut p, KEPT, "test:r", |p| {
        Value::Vec4Array(store(&mut p.f32s, &[16.0, 17.0, 18.0, 19.0]))
    });
    add(&mut entries, &mut p, KEPT, "test:s", |p| {
        Value::ColorArray(store(&mut p.f32s, &[0.5, 0.25, 0.75, 1.0]))
    });
    add(&mut entries, &mut p, KEPT, "test:t", |p| {
        Value::Mat4Array(store(&mut p.f32s, &[-0.0; 16]))
    });
    (entries, p, start, end)
}

fn without<T: Copy>(values: &[T], start: usize, end: usize) -> Vec<T> {
    values[..start]
        .iter()
        .chain(&values[end..])
        .copied()
        .collect()
}

fn assert_same_value(
    before: StoredMetadataValue,
    source: &MetadataPayloads,
    after: StoredMetadataValue,
    output: &MetadataPayloads,
) {
    assert_eq!(discriminant(&before), discriminant(&after));
    match (before, after) {
        (Value::Bool(a) | Value::BoolArray(a), Value::Bool(b) | Value::BoolArray(b)) => {
            assert_eq!(a.get(&source.bools), b.get(&output.bools));
        }
        (Value::I64(a) | Value::I64Array(a), Value::I64(b) | Value::I64Array(b)) => {
            assert_eq!(a.get(&source.i64s), b.get(&output.i64s));
        }
        (Value::U64(a) | Value::U64Array(a), Value::U64(b) | Value::U64Array(b)) => {
            assert_eq!(a.get(&source.u64s), b.get(&output.u64s));
        }
        (Value::F64(a) | Value::F64Array(a), Value::F64(b) | Value::F64Array(b)) => {
            let a: Vec<_> = a
                .get(&source.f64s)
                .unwrap()
                .iter()
                .map(|v| v.to_bits())
                .collect();
            let b: Vec<_> = b
                .get(&output.f64s)
                .unwrap()
                .iter()
                .map(|v| v.to_bits())
                .collect();
            assert_eq!(a, b);
        }
        (Value::String(a), Value::String(b)) => {
            assert_eq!(a.get_str(&source.text), b.get_str(&output.text));
        }
        (
            Value::Bytes {
                media_type: a,
                payload: x,
            },
            Value::Bytes {
                media_type: b,
                payload: y,
            },
        ) => {
            assert_eq!(a.get_str(&source.text), b.get_str(&output.text));
            assert_eq!(x.get(&source.bytes), y.get(&output.bytes));
        }
        (
            Value::Vec2(a)
            | Value::Vec3(a)
            | Value::Vec4(a)
            | Value::Color(a)
            | Value::Mat4(a)
            | Value::Vec2Array(a)
            | Value::Vec3Array(a)
            | Value::Vec4Array(a)
            | Value::ColorArray(a)
            | Value::Mat4Array(a),
            Value::Vec2(b)
            | Value::Vec3(b)
            | Value::Vec4(b)
            | Value::Color(b)
            | Value::Mat4(b)
            | Value::Vec2Array(b)
            | Value::Vec3Array(b)
            | Value::Vec4Array(b)
            | Value::ColorArray(b)
            | Value::Mat4Array(b),
        ) => {
            let a: Vec<_> = a
                .get(&source.f32s)
                .unwrap()
                .iter()
                .map(|v| v.to_bits())
                .collect();
            let b: Vec<_> = b
                .get(&output.f32s)
                .unwrap()
                .iter()
                .map(|v| v.to_bits())
                .collect();
            assert_eq!(a, b);
        }
        _ => panic!("metadata variant changed"),
    }
}

#[test]
fn deletion_compacts_every_pool_and_preserves_all_twenty_variants_and_bits() {
    let (entries, source, start, end) = mixed_fixture();
    let mut planning = Control::default();
    let mut sealed = None;
    let allocations = allocation_counter::measure(|| {
        sealed = Some(
            MetadataEditPlan::for_entries(&entries, &source, Some(DELETED), &mut planning).unwrap(),
        );
    });
    assert_eq!(allocations.count_total, 0);
    let plan = sealed.unwrap();
    assert_eq!(planning.completed, entries.len() as u64);
    assert_eq!(planning.string_bytes, 0);
    assert!(plan.rewrites_payloads());
    assert_eq!(plan.removed_entries(), 7);
    assert_eq!(plan.output_entries(), 23);
    let mut copying = Control::default();
    let output = plan.build_entries(&entries, &source, &mut copying).unwrap();
    assert_eq!(copying.completed, plan.work().unwrap());
    assert_eq!(copying.string_bytes, output.text.len() as u64);
    assert_eq!(plan.text_bytes(), output.text.len());
    assert_eq!(
        plan.capacities().map(|p| p.desired_capacity()),
        lengths(&output)
    );
    assert_eq!(
        output.text,
        format!("{}{}", &source.text[..start[0]], &source.text[end[0]..])
    );
    assert_eq!(output.bytes, without(&source.bytes, start[1], end[1]));
    assert_eq!(output.bools, without(&source.bools, start[2], end[2]));
    assert_eq!(output.i64s, without(&source.i64s, start[3], end[3]));
    assert_eq!(output.u64s, without(&source.u64s, start[4], end[4]));
    let expected = without(&source.f64s, start[5], end[5]);
    assert_eq!(
        output.f64s.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    );
    let expected = without(&source.f32s, start[6], end[6]);
    assert_eq!(
        output.f32s.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    );

    let mut cursor = MetadataEditCursor::new();
    let mut mapped = vec![];
    for &entry in entries.iter().filter(|entry| entry.owner != DELETED) {
        let after = cursor.remap(entry).unwrap();
        assert_eq!(after.owner, entry.owner);
        assert_eq!(
            after.key.get_str(&output.text),
            entry.key.get_str(&source.text)
        );
        assert_same_value(entry.value, &source, after.value, &output);
        mapped.push(after);
    }
    assert!(cursor.counts.matches(&output));
    // Re-reading the compact descriptors must consume every output pool without aliases or gaps.
    assert!(MetadataEditPlan::for_entries(&mapped, &output, None, &mut Control::default()).is_ok());
}

fn source_counts(entries: usize, p: &MetadataPayloads) -> ProfileMeteredCounts {
    ProfileMeteredCounts {
        segment_count: 1,
        resource_count: 2,
        source_count: 3,
        provenance_count: 4,
        operation_count: 5,
        evidence_count: 6,
        parent_count: 7,
        wide_count: 8,
        lineage_text_bytes: 9,
        lineage_digest_bytes: 10,
        owner_count: 11,
        attachment_count: 12,
        metadata_entry_count: entries as u32,
        metadata_text_bytes: p.text.len() as u32,
        metadata_payload_bytes: p.bytes.len() as u32,
        metadata_bool_count: p.bools.len() as u32,
        metadata_i64_count: p.i64s.len() as u32,
        metadata_u64_count: p.u64s.len() as u32,
        metadata_f64_count: p.f64s.len() as u32,
        metadata_f32_count: p.f32s.len() as u32,
    }
}

#[test]
fn output_counts_change_only_metadata_and_pure_reindex_keeps_existing_ranges() {
    let (entries, p, _, _) = mixed_fixture();
    let counts = source_counts(entries.len(), &p);
    for deleted in [None, Some(CanonicalProfileOwner::Attribute(0))] {
        let mut plan = None;
        let allocations = allocation_counter::measure(|| {
            plan = Some(
                MetadataEditPlan::for_entries(&entries, &p, deleted, &mut Control::default())
                    .unwrap(),
            );
        });
        assert_eq!(allocations.count_total, 0);
        let plan = plan.unwrap();
        assert!(!plan.rewrites_payloads());
        assert_eq!(plan.removed_entries(), 0);
        assert_eq!(plan.output_entries(), entries.len() as u32);
        assert_eq!(plan.output_counts(counts), counts);
        assert_eq!(plan.capacities().map(|p| p.desired_capacity()), lengths(&p));
    }
    let plan = MetadataEditPlan::for_entries(&entries, &p, Some(DELETED), &mut Control::default())
        .unwrap();
    let output = plan
        .build_entries(&entries, &p, &mut Control::default())
        .unwrap();
    let expected = source_counts(plan.output_entries() as usize, &output);
    assert_eq!(plan.output_counts(counts), expected);
}

fn tiny_fixture() -> (Vec<StoredMetadataEntry>, MetadataPayloads) {
    let mut p = empty_payloads();
    let mut entries = vec![];
    add(&mut entries, &mut p, DELETED, "test:a", |p| Value::Bytes {
        media_type: store_text(&mut p.text, "image/png"),
        payload: store(&mut p.bytes, &[0, 255]),
    });
    add(&mut entries, &mut p, KEPT, "test:a", |p| {
        Value::String(store_text(&mut p.text, "é😀"))
    });
    add(&mut entries, &mut p, KEPT, "test:b", |p| {
        Value::I64Array(store(&mut p.i64s, &[i64::MIN, i64::MAX]))
    });
    (entries, p)
}

#[test]
fn each_planning_and_copy_action_observes_cancellation_including_skipped_entries() {
    let (entries, p) = tiny_fixture();
    for position in 0..entries.len() as u64 {
        let mut control = Control {
            reject_before: Some(position),
            ..Control::default()
        };
        assert!(
            matches!(MetadataEditPlan::for_entries(&entries, &p, Some(DELETED), &mut control), Err(TypedStorageBuildError::Control(actual)) if actual == position)
        );
        assert_eq!(control.completed, position);
    }
    let plan = MetadataEditPlan::for_entries(&entries, &p, Some(DELETED), &mut Control::default())
        .unwrap();
    // Three descriptor scans, two six-byte keys, six UTF-8 value bytes and two i64 values.
    assert_eq!(plan.work().unwrap(), 3 + 12 + 6 + 2);
    let source_text = p.text.clone();
    let source_bytes = p.bytes.clone();
    let source_i64s = p.i64s.clone();
    for position in 0..plan.work().unwrap() {
        let mut control = Control {
            reject_before: Some(position),
            ..Control::default()
        };
        assert!(
            matches!(plan.build_entries(&entries, &p, &mut control), Err(TypedStorageBuildError::Control(actual)) if actual == position)
        );
        assert_eq!(control.completed, position);
    }
    let mut control = Control::default();
    assert!(plan.build_entries(&entries, &p, &mut control).is_ok());
    assert_eq!(control.completed, plan.work().unwrap());
    assert_eq!(control.string_bytes, 18);
    assert_eq!(p.text, source_text);
    assert_eq!(p.bytes, source_bytes);
    assert_eq!(p.i64s, source_i64s);
}

#[test]
fn deleting_the_only_owner_leaves_no_unreferenced_payload() {
    let (mut entries, p) = tiny_fixture();
    for entry in &mut entries {
        entry.owner = DELETED;
    }
    let plan = MetadataEditPlan::for_entries(&entries, &p, Some(DELETED), &mut Control::default())
        .unwrap();
    assert!(plan.rewrites_payloads());
    assert_eq!(plan.output_entries(), 0);
    assert_eq!(plan.removed_entries(), 3);
    assert_eq!(plan.work().unwrap(), 3);
    assert_eq!(plan.capacities().map(|p| p.desired_capacity()), [0; 7]);
    let mut copying = Control::default();
    let output = plan.build_entries(&entries, &p, &mut copying).unwrap();
    assert_eq!(lengths(&output), [0; 7]);
    assert_eq!(copying.completed, 3);
    assert_eq!(copying.string_bytes, 0);
    let empty = MetadataEditPlan::for_entries(&[], &output, None, &mut Control::default()).unwrap();
    assert!(!empty.rewrites_payloads());
    assert_eq!(empty.work().unwrap(), 0);
}

#[test]
fn every_empty_array_keeps_its_type_when_prior_payload_is_deleted() {
    let mut p = empty_payloads();
    let mut entries = vec![];
    add(&mut entries, &mut p, DELETED, "test:a", |p| {
        Value::Bool(store(&mut p.bools, &[1]))
    });
    add(&mut entries, &mut p, DELETED, "test:b", |p| {
        Value::I64(store(&mut p.i64s, &[1]))
    });
    add(&mut entries, &mut p, DELETED, "test:c", |p| {
        Value::U64(store(&mut p.u64s, &[1]))
    });
    add(&mut entries, &mut p, DELETED, "test:d", |p| {
        Value::F64(store(&mut p.f64s, &[1.0]))
    });
    add(&mut entries, &mut p, DELETED, "test:e", |p| {
        Value::Vec2(store(&mut p.f32s, &[1.0, 2.0]))
    });
    for (key, value) in [
        ("test:a", Value::BoolArray(TableRange::new(1, 0))),
        ("test:b", Value::I64Array(TableRange::new(1, 0))),
        ("test:c", Value::U64Array(TableRange::new(1, 0))),
        ("test:d", Value::F64Array(TableRange::new(1, 0))),
        ("test:e", Value::Vec2Array(TableRange::new(2, 0))),
        ("test:f", Value::Vec3Array(TableRange::new(2, 0))),
        ("test:g", Value::Vec4Array(TableRange::new(2, 0))),
        ("test:h", Value::ColorArray(TableRange::new(2, 0))),
        ("test:i", Value::Mat4Array(TableRange::new(2, 0))),
    ] {
        add(&mut entries, &mut p, KEPT, key, |_| value);
    }
    let plan = MetadataEditPlan::for_entries(&entries, &p, Some(DELETED), &mut Control::default())
        .unwrap();
    let output = plan
        .build_entries(&entries, &p, &mut Control::default())
        .unwrap();
    assert_eq!(lengths(&output), [9 * 6, 0, 0, 0, 0, 0, 0]);
    let mut cursor = MetadataEditCursor::new();
    for &before in entries.iter().filter(|entry| entry.owner == KEPT) {
        let after = cursor.remap(before).unwrap();
        assert_same_value(before.value, &p, after.value, &output);
    }
    assert!(cursor.counts.matches(&output));
}

#[test]
fn invalid_range_arithmetic_and_shapes_leave_the_descriptor_cursor_unchanged() {
    let mut cursor = MetadataEditCursor::new();
    let entry = StoredMetadataEntry {
        owner: KEPT,
        key: TextRange(TableRange::new(0, 6)),
        value: Value::I64(TableRange::new(0, 1)),
    };
    cursor.remap(entry).unwrap();
    let before = cursor.counts;
    for invalid in [
        StoredMetadataEntry {
            key: TextRange(TableRange::new(usize::MAX, 2)),
            ..entry
        },
        StoredMetadataEntry {
            value: Value::F64Array(TableRange::new(u32::MAX as usize, 2)),
            ..entry
        },
        StoredMetadataEntry {
            value: Value::Vec3Array(TableRange::new(0, 4)),
            ..entry
        },
        StoredMetadataEntry {
            value: Value::I64(TableRange::new(0, 0)),
            ..entry
        },
    ] {
        assert!(cursor.remap(invalid).is_none());
        assert_eq!(cursor.counts, before);
    }
    cursor.counts.text = u32::MAX;
    let before = cursor.counts;
    assert!(cursor.remap(entry).is_none());
    assert_eq!(cursor.counts, before);
}

#[test]
fn planning_rejects_bad_utf8_boundaries_gaps_and_unreferenced_payload() {
    type Corruption = fn(&mut [StoredMetadataEntry], &mut MetadataPayloads);
    let corruptions: [Corruption; 5] = [
        |entries, _| entries[0].key = TextRange(TableRange::new(usize::MAX, 2)),
        |entries, _| {
            let Value::String(range) = entries[1].value else {
                panic!("expected the UTF-8 fixture value");
            };
            entries[1].value = Value::String(TextRange(TableRange::new(range.0.start(), 1)));
        },
        |entries, _| entries[2].value = Value::I64Array(TableRange::new(1, 1)),
        |_, p| p.i64s.push(13),
        |_, p| p.bytes.push(14),
    ];
    for corrupt in corruptions {
        let (mut entries, mut p) = tiny_fixture();
        corrupt(&mut entries, &mut p);
        assert!(matches!(
            MetadataEditPlan::for_entries(&entries, &p, Some(DELETED), &mut Control::default()),
            Err(TypedStorageBuildError::Invariant)
        ));
    }
}

#[test]
fn copying_rejects_out_of_bounds_ranges_and_incomplete_source_descriptors() {
    let (entries, p) = tiny_fixture();
    let plan = MetadataEditPlan::for_entries(&entries, &p, Some(DELETED), &mut Control::default())
        .unwrap();
    let mut invalid = entries.clone();
    invalid[2].value = Value::I64Array(TableRange::new(usize::MAX, 2));
    assert!(matches!(
        plan.build_entries(&invalid, &p, &mut Control::default()),
        Err(TypedStorageBuildError::Invariant)
    ));
    let mut invalid = entries.clone();
    let Value::String(range) = invalid[1].value else {
        panic!("expected the UTF-8 fixture value");
    };
    invalid[1].value = Value::String(TextRange(TableRange::new(range.0.start(), 1)));
    assert!(matches!(
        plan.build_entries(&invalid, &p, &mut Control::default()),
        Err(TypedStorageBuildError::Invariant)
    ));
    assert!(matches!(
        plan.build_entries(&entries[..2], &p, &mut Control::default()),
        Err(TypedStorageBuildError::Invariant)
    ));
}
