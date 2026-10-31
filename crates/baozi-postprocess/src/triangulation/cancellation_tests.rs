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
    let result = triangulate_polygon(
        &positions,
        &source_corners,
        &mut scratch,
        &mut output,
        &mut || Err::<(), _>(Cancelled),
    );

    assert_eq!(result, Err(TriangulationError::Control(Cancelled)));
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
    let mut checkpoints = 0;
    triangulate_polygon(
        &positions,
        &source_corners,
        &mut scratch,
        &mut output,
        &mut || {
            checkpoints += 1;
            Ok::<_, Cancelled>(())
        },
    )
    .expect("the baseline polygon must complete");
    assert!(checkpoints > 1);

    let mut scratch = [CornerScratch::default(); POSITIONS.len()];
    let mut output = [[usize::MAX; 3]; POSITIONS.len() - 2];
    let stop = checkpoints - 1;
    let mut visited = 0;
    let result = triangulate_polygon(
        &positions,
        &source_corners,
        &mut scratch,
        &mut output,
        &mut || {
            let current = visited;
            visited += 1;
            if current == stop {
                Err(Cancelled)
            } else {
                Ok(())
            }
        },
    );

    assert_eq!(result, Err(TriangulationError::Control(Cancelled)));
    assert_eq!(visited, stop + 1);
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
        &mut || Ok::<_, Cancelled>(()),
    )
    .expect("the baseline polygon must triangulate");
    let candidate = triangles;
    let mut scratch = [CornerScratch::default(); POSITIONS.len()];
    let mut checkpoints = 0;
    validate_polygon_triangulation(
        &positions,
        &source_corners,
        &triangles,
        &mut scratch,
        &mut || {
            checkpoints += 1;
            Ok::<_, Cancelled>(())
        },
    )
    .expect("the baseline candidate must pass postflight");

    let mut scratch = [CornerScratch::default(); POSITIONS.len()];
    let stop = checkpoints / 2;
    let mut visited = 0;
    let result = validate_polygon_triangulation(
        &positions,
        &source_corners,
        &triangles,
        &mut scratch,
        &mut || {
            let current = visited;
            visited += 1;
            if current == stop {
                Err(Cancelled)
            } else {
                Ok(())
            }
        },
    );

    assert_eq!(result, Err(TriangulationError::Control(Cancelled)));
    assert_eq!(visited, stop + 1);
    assert_eq!(positions, POSITIONS);
    assert_eq!(source_corners, SOURCE_CORNERS);
    assert_eq!(triangles, candidate);
}
