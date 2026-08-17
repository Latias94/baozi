#![cfg(feature = "tracing")]

mod common;

use baozi_postprocess::{PostProcessRequests, PostProcessRunDisposition};
use common::canonical_triangle_scene;
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
fn tracing_is_fieldless_and_does_not_change_postprocess_results() {
    let scene_without_subscriber = canonical_triangle_scene();
    let scene_with_subscriber = canonical_triangle_scene();

    let without_subscriber = with_default(NoSubscriber::default(), move || {
        let scene = scene_without_subscriber;
        let generation = scene.generation();
        let canonical_block_count = scene.canonical_block_count();
        let outcome = PostProcessRequests::empty()
            .compile(scene)
            .expect("requests should compile without a subscriber")
            .execute()
            .expect("requests should succeed without a subscriber");
        (outcome, generation, canonical_block_count)
    });

    let subscriber = CapturingSubscriber::default();
    let with_subscriber = with_default(subscriber.clone(), move || {
        let scene = scene_with_subscriber;
        let generation = scene.generation();
        let canonical_block_count = scene.canonical_block_count();
        let outcome = PostProcessRequests::empty()
            .compile(scene)
            .expect("requests should compile with a subscriber")
            .execute()
            .expect("requests should succeed with a subscriber");
        (outcome, generation, canonical_block_count)
    });
    let (without_subscriber, without_generation, without_blocks) = without_subscriber;
    let (with_subscriber, with_generation, with_blocks) = with_subscriber;

    assert_eq!(
        subscriber.captured_spans(),
        [
            CapturedSpan {
                name: "baozi.postprocess.compile",
                fields: Vec::new(),
            },
            CapturedSpan {
                name: "baozi.postprocess.execute",
                fields: Vec::new(),
            },
        ]
    );
    assert_eq!(
        without_subscriber.disposition(),
        PostProcessRunDisposition::Unchanged
    );
    assert_eq!(
        with_subscriber.disposition(),
        PostProcessRunDisposition::Unchanged
    );
    assert_eq!(without_subscriber.scene().generation(), without_generation);
    assert_eq!(with_subscriber.scene().generation(), with_generation);
    assert_eq!(
        without_subscriber.scene().canonical_block_count(),
        without_blocks
    );
    assert_eq!(with_subscriber.scene().canonical_block_count(), with_blocks);
    assert_eq!(
        without_subscriber.report().resources(),
        with_subscriber.report().resources()
    );
}
