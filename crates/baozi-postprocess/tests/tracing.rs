#![cfg(feature = "tracing")]

use baozi_core::{SceneBuilder, ValidatedScene, validate_scene};
use baozi_postprocess::{PostProcessPipeline, PostProcessRunDisposition};
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
    let input = root_only_scene();
    let pipeline = PostProcessPipeline::new([]).expect("empty pipeline should compile");

    let without_subscriber = with_default(NoSubscriber::default(), || {
        pipeline
            .run(&input)
            .expect("pipeline should succeed without a subscriber")
    });

    let subscriber = CapturingSubscriber::default();
    let with_subscriber = with_default(subscriber.clone(), || {
        pipeline
            .run(&input)
            .expect("pipeline should succeed with a subscriber")
    });

    assert_eq!(
        subscriber.captured_spans(),
        [CapturedSpan {
            name: "baozi.postprocess",
            fields: Vec::new(),
        }]
    );
    assert_eq!(
        without_subscriber.disposition(),
        PostProcessRunDisposition::Unchanged
    );
    assert_eq!(
        with_subscriber.disposition(),
        PostProcessRunDisposition::Unchanged
    );
    assert_scene_content_eq(without_subscriber.scene(), with_subscriber.scene());
    assert_eq!(
        without_subscriber.report().ledger(),
        with_subscriber.report().ledger()
    );
}

fn root_only_scene() -> ValidatedScene {
    let builder = SceneBuilder::try_new().expect("test scene builder allocation should succeed");
    validate_scene(builder.finish_draft()).expect("root-only test scene should be valid")
}

fn assert_scene_content_eq(left: &ValidatedScene, right: &ValidatedScene) {
    assert_eq!(left.root(), right.root());
    assert_eq!(left.nodes(), right.nodes());
    assert_eq!(left.meshes(), right.meshes());
    assert_eq!(left.materials(), right.materials());
    assert_eq!(left.textures(), right.textures());
    assert_eq!(left.animations(), right.animations());
    assert_eq!(left.cameras(), right.cameras());
    assert_eq!(left.lights(), right.lights());
    assert_eq!(left.skins(), right.skins());
    assert_eq!(left.metadata(), right.metadata());
    assert_eq!(left.space(), right.space());
    assert_eq!(left.validation_stats(), right.validation_stats());
}
