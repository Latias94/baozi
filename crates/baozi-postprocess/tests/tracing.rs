#![cfg(feature = "tracing")]

mod common;

use baozi_postprocess::{
    PostProcessControl, PostProcessOperation, PostProcessRunDisposition, triangulate_primitive,
};
use common::{canonical_quad_scene, canonical_triangle_scene};
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
    for changed in [false, true] {
        let fixture = if changed {
            canonical_quad_scene
        } else {
            canonical_triangle_scene
        };
        let scene_without_subscriber = fixture();
        let scene_with_subscriber = fixture();
        let run = |scene: baozi_core::ValidatedScene| {
            let generation = scene.generation();
            let target = scene
                .meshes()
                .next()
                .unwrap()
                .primitives()
                .next()
                .unwrap()
                .id();
            let outcome = triangulate_primitive(scene, target, PostProcessControl::default())
                .expect("the direct operation should succeed regardless of tracing");
            assert_eq!(
                outcome.report().operation(),
                PostProcessOperation::Triangulate { target }
            );
            assert_eq!(outcome.report().input_generation(), generation);
            assert!(outcome.report().matches_scene(outcome.scene()));
            assert_eq!(outcome.scene().generation() != generation, changed);
            outcome
        };
        let without_subscriber =
            with_default(NoSubscriber::default(), || run(scene_without_subscriber));

        let subscriber = CapturingSubscriber::default();
        let with_subscriber = with_default(subscriber.clone(), || run(scene_with_subscriber));

        assert_eq!(
            subscriber.captured_spans(),
            [CapturedSpan {
                name: "baozi.postprocess.triangulate",
                fields: Vec::new(),
            }]
        );
        let expected = if changed {
            PostProcessRunDisposition::Changed
        } else {
            PostProcessRunDisposition::Unchanged
        };
        assert_eq!(without_subscriber.disposition(), expected);
        assert_eq!(with_subscriber.disposition(), expected);
        assert_eq!(
            without_subscriber.scene().canonical_block_count(),
            with_subscriber.scene().canonical_block_count()
        );
        let primitive = |outcome: &baozi_postprocess::PostProcessRunOutcome| {
            let primitive = outcome
                .scene()
                .meshes()
                .next()
                .unwrap()
                .primitives()
                .next()
                .unwrap();
            (
                primitive.topology(),
                primitive.vertex_count(),
                primitive.corner_count(),
                primitive.face_count(),
                primitive.indices().map(<[u32]>::to_vec),
            )
        };
        assert_eq!(primitive(&without_subscriber), primitive(&with_subscriber));
        let without_report = without_subscriber.report();
        let with_report = with_subscriber.report();
        assert_eq!(without_report.resources(), with_report.resources());
        assert_eq!(
            without_report
                .triangulation_remap()
                .map(|remap| remap.source_corner_origins()),
            with_report
                .triangulation_remap()
                .map(|remap| remap.source_corner_origins())
        );
        assert_eq!(
            without_report
                .triangulation_remap()
                .map(|remap| remap.source_face_triangle_offsets()),
            with_report
                .triangulation_remap()
                .map(|remap| remap.source_face_triangle_offsets())
        );
    }
}
