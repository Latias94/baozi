#![allow(dead_code)]

use baozi::ImportReport;
use baozi_import::{SceneSnapshot, SceneSnapshotLimits};

/// Captures one bounded canonical observation for facade integration tests.
pub fn scene_snapshot(report: &ImportReport) -> SceneSnapshot {
    let snapshot = scene_snapshot_with_limits(report, 64 * 1024, 4_096, 4_000_000);
    assert!(
        !snapshot.preview_items_truncated(),
        "facade snapshot preview unexpectedly exceeded its item bound"
    );
    snapshot
}

/// Captures a canonical observation with caller-selected preview bounds.
pub fn scene_snapshot_with_limits(
    report: &ImportReport,
    max_preview_bytes: usize,
    max_collection_items: usize,
    max_work: u64,
) -> SceneSnapshot {
    let limits = SceneSnapshotLimits::try_new(max_preview_bytes, max_collection_items, max_work)
        .expect("facade snapshot limits must be valid");
    let snapshot = report
        .scene()
        .capture_snapshot(limits)
        .expect("validated canonical Scene snapshot must succeed");
    assert!(
        !snapshot.preview_bytes_truncated(),
        "facade snapshot preview unexpectedly exceeded its byte bound"
    );
    snapshot
}

pub fn snapshot_line<'snapshot>(
    snapshot: &'snapshot SceneSnapshot,
    prefix: &str,
) -> &'snapshot str {
    snapshot
        .preview()
        .lines()
        .find(|line| line.trim_start().starts_with(prefix))
        .unwrap_or_else(|| panic!("missing canonical snapshot line: {prefix}"))
}
