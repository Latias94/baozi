use super::action_tests::record_event;
use super::*;

const POSITIONS: [[f64; 3]; 5] = [
    [0.0, 0.0, 0.0],
    [4.0, 0.0, 0.0],
    [4.0, 4.0, 0.0],
    [1.5, 1.0, 0.0],
    [0.0, 4.0, 0.0],
];

const SOURCE_CORNERS: [usize; 5] = [91, 7, 400, 18, 63];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cancelled;

#[test]
fn cancellation_before_kernel_work_preserves_candidate_storage() {
    let positions = POSITIONS;
    let source_corners = SOURCE_CORNERS;
    let mut scratch = [CornerScratch::default(); POSITIONS.len()];
    let sentinel = [[usize::MAX; 3]; POSITIONS.len() - 2];
    let mut output = sentinel;
    let mut events = Vec::new();
    let result = triangulate_polygon(
        &positions,
        &source_corners,
        &mut scratch,
        &mut output,
        &mut |event| {
            record_event(&mut events, event);
            Err::<(), _>(Cancelled)
        },
    );

    assert_eq!(result, Err(TriangulationError::Control(Cancelled)));
    assert_eq!(events, [WorkEvent::Checkpoint]);
    assert_eq!(output, sentinel);
    assert_eq!(positions, POSITIONS);
    assert_eq!(source_corners, SOURCE_CORNERS);
}

#[test]
fn cancellation_after_candidate_work_is_still_a_failed_kernel_result() {
    let positions = POSITIONS;
    let source_corners = SOURCE_CORNERS;
    let mut scratch = [CornerScratch::default(); POSITIONS.len()];
    let mut output = [[usize::MAX; 3]; POSITIONS.len() - 2];
    let mut baseline = Vec::new();
    triangulate_polygon(
        &positions,
        &source_corners,
        &mut scratch,
        &mut output,
        &mut |event| {
            record_event(&mut baseline, event);
            Ok::<_, Cancelled>(())
        },
    )
    .expect("the baseline polygon must complete");
    let candidate = output;
    assert_eq!(baseline.last(), Some(&WorkEvent::Checkpoint));
    assert_eq!(baseline[baseline.len() - 2], WorkEvent::Complete);

    let mut scratch = [CornerScratch::default(); POSITIONS.len()];
    let mut output = [[usize::MAX; 3]; POSITIONS.len() - 2];
    let mut events = Vec::new();
    let result = triangulate_polygon(
        &positions,
        &source_corners,
        &mut scratch,
        &mut output,
        &mut |event| {
            record_event(&mut events, event);
            if events.len() == baseline.len() {
                Err(Cancelled)
            } else {
                Ok(())
            }
        },
    );

    assert_eq!(result, Err(TriangulationError::Control(Cancelled)));
    assert_eq!(
        events, baseline,
        "all physical leaves completed before the final poll"
    );
    assert_eq!(
        output, candidate,
        "fully written storage remains a private failed candidate"
    );
    assert_eq!(positions, POSITIONS);
    assert_eq!(source_corners, SOURCE_CORNERS);
}

#[test]
fn cancellation_during_independent_postflight_preserves_candidate_input() {
    let positions = POSITIONS;
    let source_corners = SOURCE_CORNERS;
    let mut triangles = [[usize::MAX; 3]; POSITIONS.len() - 2];
    let mut kernel_scratch = [CornerScratch::default(); POSITIONS.len()];
    triangulate_polygon(
        &positions,
        &source_corners,
        &mut kernel_scratch,
        &mut triangles,
        &mut |_| Ok::<_, Cancelled>(()),
    )
    .expect("the baseline polygon must triangulate");
    let candidate = triangles;
    let mut scratch = [CornerScratch::default(); POSITIONS.len()];
    let mut baseline = Vec::new();
    validate_polygon_triangulation(
        &positions,
        &source_corners,
        &triangles,
        &mut scratch,
        &mut |event| {
            record_event(&mut baseline, event);
            Ok::<_, Cancelled>(())
        },
    )
    .expect("the baseline candidate must pass postflight");

    let mut scratch = [CornerScratch::default(); POSITIONS.len()];
    let stop = (baseline.len() / 2..baseline.len())
        .find(|&index| baseline[index] == WorkEvent::Begin)
        .expect("postflight has a middle leaf to reject");
    let mut events = Vec::new();
    let result = validate_polygon_triangulation(
        &positions,
        &source_corners,
        &triangles,
        &mut scratch,
        &mut |event| {
            record_event(&mut events, event);
            if events.len() == stop + 1 {
                Err(Cancelled)
            } else {
                Ok(())
            }
        },
    );

    assert_eq!(result, Err(TriangulationError::Control(Cancelled)));
    assert_eq!(
        events,
        baseline[..=stop],
        "only the admitted/completed prefix remains"
    );
    assert_eq!(
        events.last(),
        Some(&WorkEvent::Begin),
        "rejected leaf has no completion"
    );
    assert_eq!(positions, POSITIONS);
    assert_eq!(source_corners, SOURCE_CORNERS);
    assert_eq!(triangles, candidate);
}
