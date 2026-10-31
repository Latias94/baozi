use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::*;

use crate::test_fixtures as common;

#[derive(Clone)]
struct StopAtPoll {
    polls: Arc<AtomicUsize>,
    stop: usize,
    panic: bool,
}

impl baozi_operation::CancellationSignal for StopAtPoll {
    fn is_cancelled(&self) -> bool {
        let stop = self.polls.fetch_add(1, Ordering::Relaxed) >= self.stop;
        assert!(
            !(stop && self.panic),
            "injected cancellation callback panic"
        );
        stop
    }
}

fn run(stop: usize, panic: bool) -> usize {
    let (scene, evidence) =
        common::import_obj(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 1\ns 1\nf 1 2 3\nf 1 4 2\n")
            .into_parts();
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
    let control = PostProcessControl::default().with_cancellation_signal(StopAtPoll {
        polls: polls.clone(),
        stop,
        panic,
    });
    let result = generate_normals(scene, target, NormalOptions::default(), control);
    if stop == usize::MAX {
        let (scene, report) = result
            .expect("the baseline must publish normals")
            .into_parts();
        assert_eq!(report.resources().current_live_bytes(), 0);
        assert_eq!(report.resources().active_reservations(), 0);
        drop(scene);
        assert_eq!(report.normal_generation().unwrap().input_target(), target);
    } else {
        let failure = result.expect_err("the selected poll must stop generation");
        assert_eq!(failure.input().generation(), generation);
        assert!(evidence.matches_scene(failure.input()));
        let original = failure.input().primitive(target).unwrap();
        assert!(
            original
                .normals(baozi_core::AttributeDomain::Corner)
                .is_none()
        );
        assert_eq!(original.indices(), Some([0, 1, 2, 0, 3, 1].as_slice()));
        let PostProcessRunError::Normals { failure } = failure.error() else {
            panic!("a started normal operation must retain its typed failure");
        };
        assert!(
            matches!(
                failure.cause(),
                NormalCause::Control { .. } | NormalCause::FinalControl
            ),
            "poll {stop}: {failure:?}"
        );
        let resources = failure
            .resources()
            .expect("ordinary control failure keeps evidence");
        assert_eq!(resources.current_live_bytes(), 0);
        assert_eq!(resources.active_reservations(), 0);
        assert_eq!(resources.committed_canonical_bytes(), 0);
    }
    polls.load(Ordering::Relaxed)
}

#[test]
fn cancellation_at_every_poll_keeps_ordinary_failure_evidence() {
    let polls = run(usize::MAX, false);
    for stop in 3..polls {
        run(stop, false);
    }
}

#[test]
fn success_cancellation_and_callback_panics_release_every_owner() {
    let polls = run(usize::MAX, false);
    assert!(polls > 16);
    // Warm up the caught-panic machinery before measuring invocation-owned allocations.
    run(3, true);
    for panic in [false, true] {
        // Initial inspection, scratch/kernel work, activation, validation, and publication.
        for stop in [3, polls / 4, polls / 2, polls * 3 / 4, polls - 1] {
            let allocations = allocation_counter::measure(|| {
                run(stop, panic);
            });
            assert_eq!(allocations.count_current, 0, "poll {stop}, panic {panic}");
            assert_eq!(allocations.bytes_current, 0, "poll {stop}, panic {panic}");
        }
    }
    let allocations = allocation_counter::measure(|| {
        run(usize::MAX, false);
    });
    assert!(allocations.count_total > 0);
    assert_eq!(allocations.count_current, 0);
    assert_eq!(allocations.bytes_current, 0);
}

#[test]
fn caller_unwind_releases_scene_and_detached_normal_evidence() {
    let run = |detach| {
        let caught = std::panic::catch_unwind(|| {
            let scene = common::canonical_triangle_scene();
            let target = scene
                .meshes()
                .next()
                .unwrap()
                .primitives()
                .next()
                .unwrap()
                .id();
            let (scene, report) = generate_normals(
                scene,
                target,
                NormalOptions::default(),
                PostProcessControl::default(),
            )
            .unwrap()
            .into_parts();
            if detach {
                drop(scene);
                assert_eq!(report.normal_generation().unwrap().input_target(), target);
                panic!("caller unwind with a detached normal report");
            }
            std::hint::black_box((&scene, &report));
            panic!("caller unwind with Scene and normal report");
        });
        assert!(caught.is_err());
    };
    run(false);
    for detach in [false, true] {
        let allocations = allocation_counter::measure(|| run(detach));
        assert_eq!(allocations.count_current, 0);
        assert_eq!(allocations.bytes_current, 0);
    }
}
