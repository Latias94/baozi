use std::cell::Cell;

use super::*;

pub(super) fn record_event(events: &mut Vec<WorkEvent>, event: WorkEvent) {
    match event {
        WorkEvent::Begin | WorkEvent::Checkpoint => {
            assert_ne!(
                events.last(),
                Some(&WorkEvent::Begin),
                "nested leaf or poll"
            );
        }
        WorkEvent::Complete => {
            assert_eq!(events.last(), Some(&WorkEvent::Begin), "no active leaf");
        }
    }
    events.push(event);
}

#[test]
fn leaf_admission_precedes_work_and_completion_precedes_semantic_error() {
    for reject in [Some(WorkEvent::Begin), Some(WorkEvent::Complete), None] {
        let returned = Cell::new(false);
        let mut events = Vec::new();
        let result = run_action(
            &mut |event| {
                record_event(&mut events, event);
                assert_eq!(returned.get(), event == WorkEvent::Complete);
                if Some(event) == reject {
                    Err("leaf rejected")
                } else {
                    Ok(())
                }
            },
            || -> Result<(), TriangulationError<&str>> {
                returned.set(true);
                Err(PolygonError::NumericallyUnresolved.into())
            },
        );
        if reject == Some(WorkEvent::Begin) {
            assert!(!returned.get());
            assert_eq!(events, [WorkEvent::Begin]);
        } else {
            assert!(returned.get());
            assert_eq!(events, [WorkEvent::Begin, WorkEvent::Complete]);
        }
        let expected = if reject.is_some() {
            TriangulationError::Control("leaf rejected")
        } else {
            PolygonError::NumericallyUnresolved.into()
        };
        assert_eq!(result, Err(expected));
    }
}

#[test]
fn nonfinite_projection_completes_the_rejecting_leaf_without_writes() {
    let positions = [[0.0; 3], [f64::NAN, 0.0, 0.0], [1.0; 3]];
    let mut scratch = [CornerScratch::default(); 3];
    let mut events = Vec::new();
    let result = project(&positions, &mut scratch, &mut |event| {
        record_event(&mut events, event);
        Ok::<_, ()>(())
    });
    assert_eq!(result, Err(PolygonError::NonFinite { corner: 1 }.into()));
    assert_eq!(events, [WorkEvent::Begin, WorkEvent::Complete].repeat(2));
    assert!(scratch.iter().all(|corner| corner.point == [0.0; 3]));
}

#[test]
fn candidate_copy_completes_after_its_store_and_before_postflight() {
    for reject in [WorkEvent::Begin, WorkEvent::Complete] {
        let mut scratch = [CornerScratch::default(); 3];
        let mut output = [[0, 1, 2]];
        let mut events = Vec::new();
        let result = finish(&[13, 2, 41], &mut scratch, &mut output, 1.0, &mut |event| {
            record_event(&mut events, event);
            if event == reject {
                Err("copy rejected")
            } else {
                Ok(())
            }
        });
        assert_eq!(result, Err(TriangulationError::Control("copy rejected")));
        if reject == WorkEvent::Begin {
            assert_eq!(scratch[0].triangle, [0; 3]);
            assert_eq!(events, [WorkEvent::Begin]);
        } else {
            assert_eq!(
                scratch[0].triangle, output[0],
                "the store precedes completion"
            );
            assert_eq!(events, [WorkEvent::Begin, WorkEvent::Complete]);
        }
        assert_eq!(
            output,
            [[0, 1, 2]],
            "postflight and source mapping never ran"
        );
    }
}

#[test]
fn ear_leaf_unwind_has_no_completion_or_cleanup_callback() {
    let scratch = [CornerScratch {
        previous: usize::MAX,
        turn: 1.0,
        ..CornerScratch::default()
    }];
    let mut events = Vec::new();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        is_ear(0, &scratch, 1.0, &mut |event| {
            record_event(&mut events, event);
            Ok::<_, ()>(())
        })
    }));
    assert!(result.is_err());
    assert_eq!(events, [WorkEvent::Begin]);
}
