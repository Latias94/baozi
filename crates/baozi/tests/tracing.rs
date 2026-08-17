#![cfg(feature = "tracing")]

use baozi::{ImportReport, Importer, SceneSnapshotLimits, ValidatedScene};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::{NoSubscriber, with_default};
use tracing::{Event, Metadata, Subscriber};

#[derive(Clone, Debug, PartialEq, Eq)]
struct CapturedSpan {
    name: &'static str,
    fields: Vec<&'static str>,
}

#[derive(Clone, Default)]
struct CapturingSubscriber {
    spans: Arc<Mutex<Vec<CapturedSpan>>>,
    next_id: Arc<AtomicU64>,
}

impl CapturingSubscriber {
    fn captured_spans(&self) -> Vec<CapturedSpan> {
        self.spans
            .lock()
            .expect("captured span lock should remain available")
            .clone()
    }
}

impl Subscriber for CapturingSubscriber {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, attributes: &Attributes<'_>) -> Id {
        let metadata = attributes.metadata();
        let fields = metadata.fields().iter().map(|field| field.name()).collect();
        self.spans
            .lock()
            .expect("captured span lock should remain available")
            .push(CapturedSpan {
                name: metadata.name(),
                fields,
            });
        let id = self
            .next_id
            .fetch_add(1, Ordering::Relaxed)
            .checked_add(1)
            .expect("test span identifier space should remain available");
        Id::from_u64(id)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, _event: &Event<'_>) {}

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

#[test]
fn facade_tracing_is_fieldless_and_does_not_change_results() {
    let importer = Importer::new().expect("default importer should build");

    let without_subscriber = with_default(NoSubscriber::default(), || run_workflow(&importer));

    let subscriber = CapturingSubscriber::default();
    let with_subscriber = with_default(subscriber.clone(), || run_workflow(&importer));

    assert_eq!(
        subscriber.captured_spans(),
        [
            CapturedSpan {
                name: "baozi.import",
                fields: Vec::new(),
            },
            CapturedSpan {
                name: "baozi.detect",
                fields: Vec::new(),
            },
            CapturedSpan {
                name: "baozi.parse",
                fields: Vec::new(),
            },
            CapturedSpan {
                name: "baozi.validate",
                fields: Vec::new(),
            },
        ]
    );
    assert_import_eq(&without_subscriber, &with_subscriber);
}

fn run_workflow(importer: &Importer) -> ImportReport {
    importer
        .read_bytes("triangle.obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n")
        .expect("OBJ import should succeed")
}

fn assert_import_eq(left: &ImportReport, right: &ImportReport) {
    assert_eq!(left.source(), right.source());
    assert_eq!(left.format().id(), right.format().id());
    assert_eq!(left.detection_evidence(), right.detection_evidence());
    assert_eq!(left.diagnostics(), right.diagnostics());
    assert_eq!(left.diagnostic_summary(), right.diagnostic_summary());
    assert!(left.optional_dependency_evidence().is_empty());
    assert!(right.optional_dependency_evidence().is_empty());
    assert_eq!(left.ledger(), right.ledger());
    assert_eq!(left.last_completed(), right.last_completed());
    assert_scene_content_eq(left.scene(), right.scene());
}

fn assert_scene_content_eq(left: &ValidatedScene, right: &ValidatedScene) {
    let limits = SceneSnapshotLimits::try_new(16 * 1024, 64, 1_000_000)
        .expect("tracing snapshot limits must be valid");
    let left = left
        .capture_snapshot(limits)
        .expect("left validated Scene snapshot must succeed");
    let right = right
        .capture_snapshot(limits)
        .expect("right validated Scene snapshot must succeed");
    assert_eq!(left, right);
}
