use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::*;

use crate::test_fixtures as common;

#[derive(Clone)]
struct CancelAtPoll {
    polls: Arc<AtomicUsize>,
    stop: usize,
}

impl baozi_operation::CancellationSignal for CancelAtPoll {
    fn is_cancelled(&self) -> bool {
        self.polls.fetch_add(1, Ordering::Relaxed) >= self.stop
    }
}

#[test]
fn cancellation_during_work_and_publication_returns_an_usable_original_scene() {
    let run = |stop| {
        let (scene, evidence) = common::canonical_quad_import().into_parts();
        let generation = scene.generation();
        let target = scene
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap()
            .id();
        let polls = Arc::new(AtomicUsize::new(0));
        let signal = CancelAtPoll {
            polls: polls.clone(),
            stop,
        };
        let result = triangulate_primitive(
            scene,
            target,
            PostProcessControl::default().with_cancellation_signal(signal),
        );
        if stop == usize::MAX {
            drop(result.expect("the baseline must complete"));
        } else {
            let (scene, failure) = result
                .expect_err("the selected poll must cancel")
                .into_parts();
            assert_eq!(scene.generation(), generation);
            assert!(evidence.matches_scene(&scene));
            assert_eq!(
                scene.primitive(target).unwrap().topology(),
                baozi_core::TopologyKind::Polygons
            );
            let PostProcessRunError::Triangulation { failure } = failure else {
                panic!("a started operation must retain its triangulation cause");
            };
            assert!(
                matches!(
                    failure.cause(),
                    TriangulationCause::Control { .. } | TriangulationCause::FinalControl
                ),
                "{failure:?}"
            );
            let resources = failure
                .resources()
                .expect("ordinary cancellation must keep evidence");
            assert_eq!(resources.current_live_bytes(), 0);
            assert_eq!(resources.active_reservations(), 0);
            assert_eq!(resources.committed_canonical_bytes(), 0);
        }
        polls.load(Ordering::Relaxed)
    };

    let polls = run(usize::MAX);
    assert!(polls > 8);
    // Early scratch/kernel work, activation, central validation, and final publication.
    for stop in [3, polls / 4, polls / 2, polls - 1] {
        let allocations = allocation_counter::measure(|| {
            run(stop);
        });
        assert_eq!(allocations.count_current, 0, "poll {stop}");
        assert_eq!(allocations.bytes_current, 0, "poll {stop}");
    }
}
