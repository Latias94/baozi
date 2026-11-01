use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::*;
use crate::test_fixtures;
use baozi_core::{AttributeDomain, AttributeSemanticRef, SceneSnapshotLimits};

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
    let (scene, evidence) = test_fixtures::import_obj(
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvn 0 0 1\nf 1/1/1 2/2/1 3/3/1\n",
    )
    .into_parts();
    let target = scene
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap()
        .id();
    let limits = SceneSnapshotLimits::try_new(4096, 32, 100_000).unwrap();
    let before = scene.capture_snapshot(limits).unwrap();
    let generation = scene.generation();
    let polls = Arc::new(AtomicUsize::new(0));
    let result = generate_tangents(
        scene,
        target,
        TangentOptions::new(AttributeDomain::Corner, 0, AttributeDomain::Corner),
        PostProcessControl::default().with_cancellation_signal(StopAtPoll {
            polls: polls.clone(),
            stop,
            panic,
        }),
    );
    if stop == usize::MAX {
        let (scene, report) = result.expect("baseline must publish Tangents").into_parts();
        assert_eq!(report.resources().current_live_bytes(), 0);
        assert_eq!(report.resources().active_reservations(), 0);
        drop(scene);
        assert_eq!(report.tangent_generation().unwrap().input_target(), target);
    } else {
        let failure = result.expect_err("the selected poll must stop generation");
        assert_eq!(failure.input().generation(), generation);
        assert!(evidence.matches_scene(failure.input()));
        assert!(
            failure
                .input()
                .primitive(target)
                .unwrap()
                .attributes()
                .all(|a| { a.semantic() != AttributeSemanticRef::Tangent })
        );
        let after = failure.input().capture_snapshot(limits).unwrap();
        assert_eq!(after.digest(), before.digest());
        assert_eq!(after.preview(), before.preview());
        let PostProcessRunError::Tangents { failure } = failure.error() else {
            panic!("a started tangent operation must retain its typed failure");
        };
        assert!(
            matches!(
                failure.cause(),
                TangentCause::Control { .. } | TangentCause::FinalControl
            ),
            "poll {stop}: {failure:?}"
        );
        let resources = failure
            .resources()
            .expect("ordinary control failure evidence");
        assert_eq!(resources.current_live_bytes(), 0);
        assert_eq!(resources.active_reservations(), 0);
        assert_eq!(resources.committed_canonical_bytes(), 0);
    }
    polls.load(Ordering::Relaxed)
}

#[test]
fn cancellation_at_every_poll_keeps_the_input_digest_and_ordinary_evidence() {
    let polls = run(usize::MAX, false);
    for stop in 3..polls {
        run(stop, false);
    }
}

#[test]
fn success_cancellation_and_callback_panics_release_all_allocations() {
    let polls = run(usize::MAX, false);
    assert!(polls > 16);
    run(3, true);
    for panic in [false, true] {
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
