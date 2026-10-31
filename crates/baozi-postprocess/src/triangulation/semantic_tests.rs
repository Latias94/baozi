use super::*;

const POSITIONS: [[f64; 3]; 5] = [
    [0.0, 0.0, 0.0],
    [4.0, 0.0, 0.0],
    [4.0, 4.0, 0.0],
    [1.5, 1.0, 0.0],
    [0.0, 4.0, 0.0],
];

const SOURCE_CORNERS: [usize; 5] = [91, 7, 400, 18, 63];

#[test]
fn triangulation_preserves_source_identity_and_ring_semantics() {
    let positions = POSITIONS;
    let source_corners = SOURCE_CORNERS;
    let mut scratch = [CornerScratch::default(); POSITIONS.len()];
    let mut output = [[usize::MAX; 3]; POSITIONS.len() - 2];

    let written = triangulate_polygon(
        &positions,
        &source_corners,
        &mut scratch,
        &mut output,
        &mut || Ok::<_, ()>(()),
    )
    .expect("the admitted simple polygon must triangulate");

    assert_eq!(written, POSITIONS.len() - 2);
    assert_eq!(positions, POSITIONS);
    assert_eq!(source_corners, SOURCE_CORNERS);

    let ring_area = (0..POSITIONS.len())
        .map(|index| {
            let next = ring_next(index, POSITIONS.len());
            orient(POSITIONS[index], POSITIONS[next], [0.0; 3])
        })
        .sum::<f64>();
    let mut seen = [false; SOURCE_CORNERS.len()];
    for triangle in output {
        let ordinals = triangle.map(|corner| {
            SOURCE_CORNERS
                .iter()
                .position(|&source| source == corner)
                .expect("output must contain source-corner identities")
        });
        assert!(ordinals[0] != ordinals[1]);
        assert!(ordinals[1] != ordinals[2]);
        assert!(ordinals[2] != ordinals[0]);
        for ordinal in ordinals {
            seen[ordinal] = true;
        }
        assert!(
            orient(
                POSITIONS[ordinals[0]],
                POSITIONS[ordinals[1]],
                POSITIONS[ordinals[2]],
            ) * ring_area
                > 0.0
        );
    }
    assert!(seen.into_iter().all(|present| present));

    let mut postflight_scratch = [CornerScratch::default(); POSITIONS.len()];
    validate_polygon_triangulation(
        &positions,
        &source_corners,
        &output,
        &mut postflight_scratch,
        &mut || Ok::<_, ()>(()),
    )
    .expect("the emitted candidate must satisfy independent postflight");
}
