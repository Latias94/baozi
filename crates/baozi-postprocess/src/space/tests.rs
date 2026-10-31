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

fn options() -> SpaceConversionOptions {
    SpaceConversionOptions {
        target: TargetSpace::try_new(
            baozi_core::Handedness::Left,
            baozi_core::Axis::PositiveY,
            baozi_core::Axis::NegativeZ,
            0.01,
            baozi_core::FrontFaceWinding::CounterClockwise,
        )
        .unwrap(),
        source_assumptions: Some(TargetSpace::STANDARD),
    }
}

#[test]
fn conversion_cancellation_during_planning_validation_and_publication_releases_all_storage() {
    let run = |stop| {
        let (scene, evidence) = common::canonical_triangle_import().into_parts();
        let generation = scene.generation();
        let polls = Arc::new(AtomicUsize::new(0));
        let signal = CancelAtPoll {
            polls: polls.clone(),
            stop,
        };
        let result = convert_scene_space(
            scene,
            options(),
            PostProcessControl::default().with_cancellation_signal(signal),
        );
        if stop == usize::MAX {
            drop(result.expect("baseline must complete"));
        } else {
            let (scene, error) = result
                .expect_err("the selected poll must cancel")
                .into_parts();
            assert_eq!(scene.generation(), generation);
            assert!(evidence.matches_scene(&scene));
            assert_eq!(scene.space().unit_scale_to_meters, None);
            let PostProcessRunError::SpaceConversion { failure } = error else {
                panic!("space conversion failure");
            };
            assert!(
                matches!(
                    failure.cause(),
                    SpaceConversionCause::Control { .. } | SpaceConversionCause::FinalControl
                ),
                "{failure:?}"
            );
            let resources = failure
                .resources()
                .expect("ordinary cancellation retains terminal evidence");
            assert_eq!(resources.current_live_bytes(), 0);
            assert_eq!(resources.active_reservations(), 0);
            assert_eq!(resources.committed_canonical_bytes(), 0);
        }
        polls.load(Ordering::Relaxed)
    };
    let polls = run(usize::MAX);
    assert!(polls > 8);
    for stop in [3, polls / 4, polls / 2, polls - 1] {
        let measured = allocation_counter::measure(|| {
            run(stop);
        });
        assert_eq!(measured.count_current, 0, "poll {stop}");
        assert_eq!(measured.bytes_current, 0, "poll {stop}");
    }
}

#[test]
fn ordinary_host_unwind_drops_converted_scene_and_detached_report() {
    // Isolate first-use std panic and output-capture allocations. Keep the entire
    // first import and conversion inside the owner measurement below.
    let panic_runtime = allocation_counter::measure(|| {
        let panic = std::panic::catch_unwind(|| {
            panic!("ordinary host panic after successful conversion");
        });
        assert!(panic.is_err());
        drop(panic);
    });
    eprintln!(
        "bare std catch/drop: count_current={}, bytes_current={}",
        panic_runtime.count_current, panic_runtime.bytes_current
    );

    let measured = allocation_counter::measure(|| {
        let panic = std::panic::catch_unwind(|| {
            let (scene, evidence) = common::canonical_triangle_import().into_parts();
            let (scene, report) =
                convert_scene_space(scene, options(), PostProcessControl::default())
                    .unwrap()
                    .into_parts();
            drop(evidence);
            assert!(report.matches_scene(&scene));
            panic!("ordinary host panic after successful conversion");
        });
        assert!(panic.is_err());
        drop(panic);
    });
    assert_eq!(measured.count_current, 0);
    assert_eq!(measured.bytes_current, 0);
}
