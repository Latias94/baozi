use super::*;

const SQUARE: [[f64; 3]; 4] = [
    [0.0, 0.0, 0.0],
    [2.0, 0.0, 0.0],
    [2.0, 2.0, 0.0],
    [0.0, 2.0, 0.0],
];

fn validate(
    positions: &[[f64; 3]],
    sources: &[usize],
    triangles: &[[usize; 3]],
) -> Result<(), TriangulationError<()>> {
    let mut scratch = vec![CornerScratch::default(); positions.len()];
    validate_polygon_triangulation(positions, sources, triangles, &mut scratch, &mut || Ok(()))
}

#[test]
fn accepts_hand_authored_disks_without_running_ear_clipping() {
    let sources = [31, 8, 52, 4];
    let triangles = [[31, 8, 52], [31, 52, 4]];
    assert_eq!(validate(&SQUARE, &sources, &triangles), Ok(()));
    assert_eq!(
        validate(&SQUARE, &sources, &[[31, 8, 4], [8, 52, 4]]),
        Ok(())
    );
    let tilted = SQUARE.map(|[x, y, _]| [3.0 + x, y - 2.0, 2.0 * x - y]);
    assert_eq!(validate(&tilted, &sources, &triangles), Ok(()));
    let mut reversed = tilted;
    reversed.reverse();
    let mut reversed_sources = sources;
    reversed_sources.reverse();
    let clockwise = triangles.map(|[a, b, c]| [a, c, b]);
    assert_eq!(validate(&reversed, &reversed_sources, &clockwise), Ok(()));
}

#[test]
fn accepts_a_concave_disk_and_a_retained_straight_boundary_corner() {
    let concave = [
        [0.0, 0.0, 0.0],
        [3.0, 0.0, 0.0],
        [3.0, 3.0, 0.0],
        [1.5, 1.0, 0.0],
        [0.0, 3.0, 0.0],
    ];
    let sources = [0, 1, 2, 3, 4];
    assert_eq!(
        validate(&concave, &sources, &[[1, 2, 3], [0, 1, 3], [0, 3, 4]]),
        Ok(())
    );
    let straight = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [2.0, 2.0, 0.0],
        [0.0, 2.0, 0.0],
    ];
    assert_eq!(
        validate(&straight, &sources, &[[4, 0, 1], [4, 1, 2], [2, 3, 4]]),
        Ok(())
    );
}

#[test]
fn area_and_corner_coverage_do_not_admit_crossing_diagonals() {
    // Both triangles have the source winding, cover all corners, and their areas
    // sum to the square's area. They cross along diagonals and duplicate edge 0->1.
    let bad = [[0, 1, 2], [0, 1, 3]];
    assert_eq!(
        validate(&SQUARE, &[0, 1, 2, 3], &bad),
        Err(PolygonError::InvalidTriangulation.into())
    );
}

#[test]
fn duplicate_triangle_can_match_count_area_and_coverage_but_is_rejected() {
    let hexagon = [
        [0.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [3.0, 1.0, 0.0],
        [2.0, 2.0, 0.0],
        [0.0, 2.0, 0.0],
        [-1.0, 1.0, 0.0],
    ];
    // Twice-areas 2 + 2 + 4 + 4 match the ring's 12. Every source corner occurs.
    let bad = [[0, 1, 2], [0, 1, 2], [0, 1, 3], [1, 4, 5]];
    assert_eq!(
        validate(&hexagon, &[0, 1, 2, 3, 4, 5], &bad),
        Err(PolygonError::InvalidTriangulation.into())
    );
}

#[test]
fn missing_corner_extra_triangle_and_unknown_corner_are_rejected() {
    let ring = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [2.0, 1.0, 0.0],
        [0.0, 2.0, 0.0],
    ];
    // Count, winding and total area still match, but collinear corner 1 is omitted.
    let missing = [[0, 2, 3], [0, 2, 3], [2, 3, 4]];
    assert_eq!(
        validate(&ring, &[0, 1, 2, 3, 4], &missing),
        Err(PolygonError::InvalidTriangulation.into())
    );
    assert_eq!(
        validate(&SQUARE, &[0, 1, 2, 3], &[[0, 1, 2], [0, 2, 3], [0, 1, 3]]),
        Err(PolygonError::InvalidTriangulation.into())
    );
    assert_eq!(
        validate(&SQUARE, &[0, 1, 2, 3], &[[0, 1, 2], [0, 2, usize::MAX]]),
        Err(PolygonError::InvalidTriangulation.into())
    );
}

#[test]
fn directed_incidence_rejects_an_unpaired_diagonal_without_geometry_heuristics() {
    let mut scratch = [CornerScratch::default(); 4];
    scratch[0].triangle = [0, 1, 2];
    scratch[1].triangle = [0, 1, 3];
    // Diagonal 2->0 has no 0->2 partner; this is a combinatorial rejection even
    // without projected points, area arithmetic, or intersection tests.
    assert_eq!(
        validate_edge_incidence(&mut scratch, 2, &mut || Ok::<_, ()>(())),
        Err(PolygonError::InvalidTriangulation.into())
    );
}

#[test]
fn connectivity_requires_a_single_disk_instead_of_disjoint_triangle_islands() {
    let mut scratch = [CornerScratch::default(); 6];
    scratch[0].triangle = [0, 1, 2];
    scratch[1].triangle = [3, 4, 5];
    assert_eq!(
        validate_disk_connectivity(&mut scratch, 2, &mut || Ok::<_, ()>(())),
        Err(PolygonError::InvalidTriangulation.into())
    );
}

#[test]
fn strictly_nested_triangles_are_overlap_even_without_edge_crossings() {
    let points = [
        [-1.0, -1.0, 0.0],
        [1.0, -1.0, 0.0],
        [0.0, 1.0, 0.0],
        [-0.25, -0.5, 0.0],
        [0.25, -0.5, 0.0],
        [0.0, 0.0, 0.0],
    ];
    let mut scratch = points.map(|point| CornerScratch {
        point,
        ..CornerScratch::default()
    });
    scratch[0].triangle = [0, 1, 2];
    scratch[1].triangle = [3, 4, 5];
    assert_eq!(
        validate_output_geometry(&scratch, 2, 1.0, &mut || Ok::<_, ()>(())),
        Err(PolygonError::InvalidTriangulation.into())
    );
}

#[test]
fn contact_in_edge_interior_and_partial_shared_edges_are_rejected() {
    let points = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.5, 0.0, 0.0],
        [1.0, -0.5, 0.0],
    ];
    let mut scratch = points.map(|point| CornerScratch {
        point,
        ..CornerScratch::default()
    });
    scratch[0].triangle = [0, 1, 2];
    scratch[1].triangle = [3, 4, 1];
    assert_eq!(
        validate_output_geometry(&scratch, 2, 1.0, &mut || Ok::<_, ()>(())),
        Err(PolygonError::InvalidTriangulation.into())
    );
    scratch[1].triangle = [0, 4, 3];
    assert_eq!(
        validate_output_geometry(&scratch, 2, 1.0, &mut || Ok::<_, ()>(())),
        Err(PolygonError::InvalidTriangulation.into())
    );
}

#[test]
fn independent_postflight_abort_never_changes_the_candidate_or_scratch_tail() {
    let triangles = [[0, 1, 2], [0, 2, 3]];
    let mut scratch = [CornerScratch::default(); 5];
    scratch[4].point = [42.0; 3];
    let mut checkpoints = 0_usize;
    validate_polygon_triangulation(
        &SQUARE,
        &[0, 1, 2, 3],
        &triangles,
        &mut scratch,
        &mut || {
            checkpoints += 1;
            Ok::<_, usize>(())
        },
    )
    .unwrap();
    for stop in 0..checkpoints {
        let candidate = triangles;
        let mut visited = 0;
        let result = validate_polygon_triangulation(
            &SQUARE,
            &[0, 1, 2, 3],
            &candidate,
            &mut scratch,
            &mut || {
                let current = visited;
                visited += 1;
                if current == stop { Err(stop) } else { Ok(()) }
            },
        );
        assert_eq!(result, Err(TriangulationError::Control(stop)));
        assert_eq!(visited, stop + 1);
        assert_eq!(candidate, triangles);
        assert_eq!(scratch[4].point, [42.0; 3]);
    }
}
