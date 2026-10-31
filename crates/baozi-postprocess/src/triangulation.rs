//! Private, allocation-free triangulation of one simple polygon without holes.
//!
//! Ear maintenance follows the quadratic algorithm in David Eberly, "Triangulation by
//! Ear Clipping", section 2 (2024-08-03):
//! <https://www.geometrictools.com/Documentation/TriangulationByEarClipping.pdf>.
//! Dominant-normal projection is also used by Assimp's TriangulateProcess.cpp.
//! There is no point removal, intersection repair, hole bridging, or fallback fan.

// Coordinates are translated and divided by the largest displacement component.
// Thus every component is in [-1, 1]. Tolerances below are in that normalized space:
// distance is relative to that extent, and determinants are relative to its square.
// They deliberately exceed the roundoff of the fixed-size f64 predicates. Uncertain
// intersections/ears are rejected conservatively; these are not exact predicates.
const DISTANCE_TOLERANCE: f64 = 128.0 * f64::EPSILON;
const DETERMINANT_TOLERANCE: f64 = 512.0 * f64::EPSILON;
const PLANAR_TOLERANCE: f64 = 1.0e-10;

/// One caller-admitted scratch slot per input corner. Its contents are disposable.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CornerScratch {
    // Normalized 3D point during plane admission; then [projected_x, projected_y, 0].
    point: [f64; 3],
    // Ear links; postflight reuses previous as a queue entry and next as boundary hits.
    previous: usize,
    next: usize,
    turn: f64,
    // At most n - 2 ordinal triples occupy the existing n admitted corner slots.
    triangle: [usize; 3],
    // Ear membership / postflight triangle reachability, and ear / vertex coverage.
    active: bool,
    ear: bool,
}

/// Diagnostic indices are input ring ordinals; the caller resolves their source owners.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolygonError {
    TooFewCorners,
    CornerCountMismatch,
    InsufficientScratch,
    InsufficientOutput,
    NonFinite {
        corner: usize,
    },
    RepeatedSourceCorner {
        first: usize,
        second: usize,
    },
    CoincidentCorners {
        first: usize,
        second: usize,
    },
    Degenerate,
    NonPlanar {
        corner: usize,
    },
    NonSimple {
        first_edge: usize,
        second_edge: usize,
    },
    NumericallyUnresolved,
    InvalidTriangulation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TriangulationError<C> {
    Polygon(PolygonError),
    Control(C),
}

impl<C> From<PolygonError> for TriangulationError<C> {
    fn from(value: PolygonError) -> Self {
        Self::Polygon(value)
    }
}

/// Triangulates an ordered ring, returning `positions.len() - 2` source-corner triples.
///
/// `source_corners[i]` is the caller's owner-local corner identity for `positions[i]`,
/// not a vertex identity. Identities must be distinct; their bounds belong to the caller.
/// Every input corner is retained, including corners on straight boundary chains.
/// Each triangle follows the original ring's winding. Repeating the closing corner is
/// invalid, and this interface cannot represent holes.
///
/// The caller admits and initializes at least `n` scratch slots and `n - 2` output
/// slots before entry. This function allocates nothing. Time is O(n^2), scratch O(n).
/// Every input-dependent loop iteration calls `control` before bounded arithmetic;
/// the caller charges work and polls control there. Fixed-size vector math needs no
/// separate checkpoint. Control errors propagate without interpretation, including
/// at a final checkpoint after the last output write.
///
/// Both writable slices MUST remain private candidate storage until `Ok`. On ANY
/// error, including control failure, their contents are unspecified and must be
/// discarded by the caller's edit transaction. Input slices are never modified.
/// Capacity/shape errors precede all writes. Slice tails beyond the required lengths
/// are untouched, even on failure.
pub(crate) fn triangulate_polygon<C>(
    positions: &[[f64; 3]],
    source_corners: &[usize],
    scratch: &mut [CornerScratch],
    output: &mut [[usize; 3]],
    control: &mut impl FnMut() -> Result<(), C>,
) -> Result<usize, TriangulationError<C>> {
    let count = positions.len();
    if count < 3 {
        return Err(PolygonError::TooFewCorners.into());
    }
    if source_corners.len() != count {
        return Err(PolygonError::CornerCountMismatch.into());
    }
    let scratch = scratch
        .get_mut(..count)
        .ok_or(PolygonError::InsufficientScratch)?;
    let output = output
        .get_mut(..count - 2)
        .ok_or(PolygonError::InsufficientOutput)?;

    project(positions, scratch, control)?;
    let signed_area = admit_ring(source_corners, scratch, control)?;
    let winding = signed_area.signum();

    for index in 0..count {
        control().map_err(TriangulationError::Control)?;
        scratch[index].ear = is_ear(index, scratch, winding, control)?;
    }

    let mut remaining = count;
    let mut cursor = 0;
    let mut written = 0;
    while remaining > 3 {
        control().map_err(TriangulationError::Control)?;
        let mut tip = None;
        for _ in 0..remaining {
            control().map_err(TriangulationError::Control)?;
            if scratch[cursor].ear {
                // Recheck before emitting: tolerance-based caching may conservatively
                // retain a stale classification, but must never admit a stale ear.
                if is_ear(cursor, scratch, winding, control)? {
                    tip = Some(cursor);
                    break;
                }
                scratch[cursor].ear = false;
            }
            cursor = scratch[cursor].next;
        }
        let tip = tip.ok_or(PolygonError::NumericallyUnresolved)?;
        let previous = scratch[tip].previous;
        let next = scratch[tip].next;
        // Until postflight these are working ordinals, never public corner identities.
        output[written] = [previous, tip, next];
        written += 1;
        scratch[previous].next = next;
        scratch[next].previous = previous;
        scratch[tip].active = false;
        scratch[tip].ear = false;
        remaining -= 1;
        cursor = next;

        // Both turns must be current before either ear test examines reflex corners.
        for index in [previous, next] {
            scratch[index].turn = corner_turn(index, scratch);
        }
        if remaining > 3 {
            for index in [previous, next] {
                scratch[index].ear = is_ear(index, scratch, winding, control)?;
            }
        }
    }
    control().map_err(TriangulationError::Control)?;
    output[written] = [scratch[cursor].previous, cursor, scratch[cursor].next];

    finish(source_corners, scratch, output, signed_area, control)?;
    control().map_err(TriangulationError::Control)?;
    Ok(output.len())
}

/// Independently checks source-corner triples against one ordered simple polygon.
///
/// Unlike the internal clipping postflight, this entry point also admits the source
/// ring and resolves arbitrary distinct `source_corners` to ring ordinals. It neither
/// calls ear clipping nor changes the candidate triples. It uses exactly `n` caller
/// scratch slots, no allocation, and O(n^2) time for `n - 2` candidate triangles.
///
/// Checks include winding, nonzero area, coverage, area agreement, directed boundary
/// and opposite interior incidence, connected disk topology, crossings, T-junctions,
/// collinear edge overlap, and triangle containment. Float predicates use the same
/// conservative normalized tolerances as clipping; near contacts can be rejected.
///
/// Every input-dependent iteration calls `control`; each call covers only a bounded
/// amount of arithmetic (at most nine edge pairs and six point tests). A conservative
/// bound for this whole entry point is `12*n*n + 32*n + 1` calls, computed with checked
/// arithmetic by the caller. Scratch is disposable on any result, and its tail beyond
/// `n` is untouched. Input and candidate slices are always unchanged, including abort.
pub(crate) fn validate_polygon_triangulation<C>(
    positions: &[[f64; 3]],
    source_corners: &[usize],
    triangles: &[[usize; 3]],
    scratch: &mut [CornerScratch],
    control: &mut impl FnMut() -> Result<(), C>,
) -> Result<(), TriangulationError<C>> {
    let count = positions.len();
    if count < 3 {
        return Err(PolygonError::TooFewCorners.into());
    }
    if source_corners.len() != count {
        return Err(PolygonError::CornerCountMismatch.into());
    }
    if triangles.len() != count - 2 {
        return Err(PolygonError::InvalidTriangulation.into());
    }
    let scratch = scratch
        .get_mut(..count)
        .ok_or(PolygonError::InsufficientScratch)?;
    project(positions, scratch, control)?;
    let signed_area = admit_ring(source_corners, scratch, control)?;
    for (index, triangle) in triangles.iter().enumerate() {
        control().map_err(TriangulationError::Control)?;
        let mut ordinals = [0; 3];
        for lane in 0..3 {
            let mut ordinal = None;
            for (source_index, &source) in source_corners.iter().enumerate() {
                control().map_err(TriangulationError::Control)?;
                if source == triangle[lane] {
                    ordinal = Some(source_index);
                    break;
                }
            }
            ordinals[lane] = ordinal.ok_or(PolygonError::InvalidTriangulation)?;
        }
        scratch[index].triangle = ordinals;
    }
    validate_projected_triangulation(scratch, triangles.len(), signed_area, control)
}

fn project<C>(
    positions: &[[f64; 3]],
    scratch: &mut [CornerScratch],
    control: &mut impl FnMut() -> Result<(), C>,
) -> Result<(), TriangulationError<C>> {
    let mut magnitude = 0.0_f64;
    for (index, point) in positions.iter().enumerate() {
        control().map_err(TriangulationError::Control)?;
        if !point[0].is_finite() || !point[1].is_finite() || !point[2].is_finite() {
            return Err(PolygonError::NonFinite { corner: index }.into());
        }
        magnitude = magnitude.max(point[0].abs().max(point[1].abs()).max(point[2].abs()));
    }

    // A power-of-two prescale avoids overflowing opposite-sign extreme coordinates.
    // Otherwise subtract first, preserving small representable offsets at large origins.
    let prescale = if magnitude > f64::MAX / 4.0 {
        0.25
    } else {
        1.0
    };
    let origin = positions[0].map(|value| value * prescale);
    let mut extent = 0.0_f64;
    let count = positions.len();
    for (index, point) in positions.iter().enumerate() {
        control().map_err(TriangulationError::Control)?;
        let point = subtract3(point.map(|value| value * prescale), origin);
        extent = extent.max(point[0].abs().max(point[1].abs()).max(point[2].abs()));
        scratch[index] = CornerScratch {
            point,
            previous: if index == 0 { count - 1 } else { index - 1 },
            next: if index == count - 1 { 0 } else { index + 1 },
            active: true,
            ..CornerScratch::default()
        };
    }
    if extent == 0.0 {
        return Err(PolygonError::Degenerate.into());
    }

    let mut baseline = [0.0; 3];
    let mut longest = 0.0;
    for corner in scratch.iter_mut() {
        control().map_err(TriangulationError::Control)?;
        corner.point = corner.point.map(|value| value / extent);
        let length = dot3(corner.point, corner.point);
        if length > longest {
            longest = length;
            baseline = corner.point;
        }
    }
    // The widest triangle using the longest origin baseline avoids cancellation of
    // an area-sum normal on crossing rings; simplicity is checked independently below.
    let mut normal = [0.0; 3];
    let mut widest = 0.0;
    for corner in scratch.iter() {
        control().map_err(TriangulationError::Control)?;
        let candidate = cross3(baseline, corner.point);
        let area = dot3(candidate, candidate);
        if area > widest {
            widest = area;
            normal = candidate;
        }
    }
    let normal_length = widest.sqrt();
    if normal_length <= DETERMINANT_TOLERANCE {
        return Err(PolygonError::Degenerate.into());
    }
    normal = normal.map(|value| value / normal_length);
    let axes = if normal[0].abs() >= normal[1].abs() && normal[0].abs() >= normal[2].abs() {
        [1, 2]
    } else if normal[1].abs() >= normal[2].abs() {
        [2, 0]
    } else {
        [0, 1]
    };
    for (index, corner) in scratch.iter_mut().enumerate() {
        control().map_err(TriangulationError::Control)?;
        if dot3(normal, corner.point).abs() > PLANAR_TOLERANCE {
            return Err(PolygonError::NonPlanar { corner: index }.into());
        }
        corner.point = [corner.point[axes[0]], corner.point[axes[1]], 0.0];
    }
    Ok(())
}

fn admit_ring<C>(
    source_corners: &[usize],
    scratch: &mut [CornerScratch],
    control: &mut impl FnMut() -> Result<(), C>,
) -> Result<f64, TriangulationError<C>> {
    let count = scratch.len();
    let mut area = AreaSum::default();
    for first in 0..count {
        control().map_err(TriangulationError::Control)?;
        let a = scratch[first].point;
        let b = scratch[scratch[first].next].point;
        area.add(a[0] * b[1] - a[1] * b[0]);
        scratch[first].turn = corner_turn(first, scratch);
        let previous = scratch[scratch[first].previous].point;
        if scratch[first].turn.abs() <= DETERMINANT_TOLERANCE
            && dot3(subtract3(a, previous), subtract3(b, a)) <= 0.0
        {
            return Err(PolygonError::NonSimple {
                first_edge: scratch[first].previous,
                second_edge: first,
            }
            .into());
        }

        for second in first + 1..count {
            control().map_err(TriangulationError::Control)?;
            if source_corners[first] == source_corners[second] {
                return Err(PolygonError::RepeatedSourceCorner { first, second }.into());
            }
            let c = scratch[second].point;
            let distance = subtract3(a, c);
            if dot3(distance, distance) <= DISTANCE_TOLERANCE * DISTANCE_TOLERANCE {
                return Err(PolygonError::CoincidentCorners { first, second }.into());
            }
            if second == first + 1 || (first == 0 && second == count - 1) {
                continue;
            }
            let d = scratch[scratch[second].next].point;
            if segments_may_intersect(a, b, c, d) {
                return Err(PolygonError::NonSimple {
                    first_edge: first,
                    second_edge: second,
                }
                .into());
            }
        }
    }
    let area = area.total();
    if area.abs() <= DETERMINANT_TOLERANCE * count as f64 {
        return Err(PolygonError::Degenerate.into());
    }
    Ok(area)
}

fn is_ear<C>(
    index: usize,
    scratch: &[CornerScratch],
    winding: f64,
    control: &mut impl FnMut() -> Result<(), C>,
) -> Result<bool, TriangulationError<C>> {
    let corner = &scratch[index];
    if winding * corner.turn <= DETERMINANT_TOLERANCE {
        return Ok(false);
    }
    let a = scratch[corner.previous].point;
    let b = corner.point;
    let c = scratch[corner.next].point;
    for (other_index, other) in scratch.iter().enumerate() {
        control().map_err(TriangulationError::Control)?;
        if !other.active
            || other_index == index
            || other_index == corner.previous
            || other_index == corner.next
            || winding * other.turn > DETERMINANT_TOLERANCE
        {
            continue;
        }
        // Reflex AND flat corners block an ear, including points on its diagonal.
        // Keeping flat blockers retains every source corner on boundary chains.
        if winding * orient(a, b, other.point) >= -DETERMINANT_TOLERANCE
            && winding * orient(b, c, other.point) >= -DETERMINANT_TOLERANCE
            && winding * orient(c, a, other.point) >= -DETERMINANT_TOLERANCE
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn finish<C>(
    source_corners: &[usize],
    scratch: &mut [CornerScratch],
    output: &mut [[usize; 3]],
    signed_area: f64,
    control: &mut impl FnMut() -> Result<(), C>,
) -> Result<(), TriangulationError<C>> {
    for (index, &triangle) in output.iter().enumerate() {
        control().map_err(TriangulationError::Control)?;
        scratch[index].triangle = triangle;
    }
    validate_projected_triangulation(scratch, output.len(), signed_area, control)?;
    for triangle in output.iter_mut() {
        control().map_err(TriangulationError::Control)?;
        *triangle = triangle.map(|index| source_corners[index]);
    }
    Ok(())
}

/// Uses only projected points and candidate triples, independent of the ear links.
/// For n slots this phase takes at most `6*n*n + 16*n + 1` controlled units.
fn validate_projected_triangulation<C>(
    scratch: &mut [CornerScratch],
    triangle_count: usize,
    signed_area: f64,
    control: &mut impl FnMut() -> Result<(), C>,
) -> Result<(), TriangulationError<C>> {
    let count = scratch.len();
    if count < 3 || triangle_count != count - 2 {
        return Err(PolygonError::InvalidTriangulation.into());
    }
    for corner in scratch.iter_mut() {
        control().map_err(TriangulationError::Control)?;
        corner.ear = false;
        corner.active = false;
        corner.next = 0;
    }
    let mut area = AreaSum::default();
    for index in 0..triangle_count {
        control().map_err(TriangulationError::Control)?;
        let [a, b, c] = scratch[index].triangle;
        if a >= count || b >= count || c >= count || a == b || b == c || c == a {
            return Err(PolygonError::InvalidTriangulation.into());
        }
        let triangle_area = orient(scratch[a].point, scratch[b].point, scratch[c].point);
        if signed_area.signum() * triangle_area <= DETERMINANT_TOLERANCE {
            return Err(PolygonError::InvalidTriangulation.into());
        }
        area.add(triangle_area);
        scratch[a].ear = true;
        scratch[b].ear = true;
        scratch[c].ear = true;
    }
    for corner in scratch.iter() {
        control().map_err(TriangulationError::Control)?;
        if !corner.ear {
            return Err(PolygonError::InvalidTriangulation.into());
        }
    }
    if (area.total() - signed_area).abs() > DETERMINANT_TOLERANCE * scratch.len() as f64 {
        return Err(PolygonError::InvalidTriangulation.into());
    }

    validate_output_geometry(scratch, triangle_count, signed_area.signum(), control)?;
    let edges = validate_edge_incidence(scratch, triangle_count, control)?;
    // All retained vertices are boundary vertices of this one no-hole ring. Euler
    // and dual connectivity are checked separately from geometric embedding/incidence.
    if count
        .checked_add(triangle_count)
        .and_then(|value| value.checked_sub(edges))
        != Some(1)
    {
        return Err(PolygonError::InvalidTriangulation.into());
    }
    validate_disk_connectivity(scratch, triangle_count, control)?;
    control().map_err(TriangulationError::Control)?;
    Ok(())
}

fn validate_output_geometry<C>(
    scratch: &[CornerScratch],
    triangle_count: usize,
    winding: f64,
    control: &mut impl FnMut() -> Result<(), C>,
) -> Result<(), TriangulationError<C>> {
    for first in 0..triangle_count {
        control().map_err(TriangulationError::Control)?;
        let a = scratch[first].triangle;
        for second in first + 1..triangle_count {
            control().map_err(TriangulationError::Control)?;
            let b = scratch[second].triangle;
            for first_edge in triangle_edges(a) {
                for second_edge in triangle_edges(b) {
                    if edges_improperly_intersect(first_edge, second_edge, scratch) {
                        return Err(PolygonError::InvalidTriangulation.into());
                    }
                }
            }
            if a.iter().all(|index| b.contains(index)) {
                return Err(PolygonError::InvalidTriangulation.into());
            }
            // Edge tests alone miss strict containment. Skip equal source vertices,
            // but treat every other near-boundary point conservatively as contact.
            for (inner, outer) in [(a, b), (b, a)] {
                for index in inner {
                    if !outer.contains(&index)
                        && point_may_be_in_triangle(scratch[index].point, outer, scratch, winding)
                    {
                        return Err(PolygonError::InvalidTriangulation.into());
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_edge_incidence<C>(
    scratch: &mut [CornerScratch],
    triangle_count: usize,
    control: &mut impl FnMut() -> Result<(), C>,
) -> Result<usize, TriangulationError<C>> {
    let mut edges = 0_usize;
    let count = scratch.len();
    for first in 0..triangle_count {
        control().map_err(TriangulationError::Control)?;
        for [a, b] in triangle_edges(scratch[first].triangle) {
            if ring_next(a, count) == b {
                if scratch[a].next != 0 {
                    return Err(PolygonError::InvalidTriangulation.into());
                }
                scratch[a].next = 1;
            } else {
                if ring_next(b, count) == a {
                    return Err(PolygonError::InvalidTriangulation.into());
                }
                let mut same = 0;
                let mut opposite = 0;
                for other in &scratch[..triangle_count] {
                    control().map_err(TriangulationError::Control)?;
                    for [c, d] in triangle_edges(other.triangle) {
                        same += usize::from(a == c && b == d);
                        opposite += usize::from(a == d && b == c);
                    }
                    if same > 1 || opposite > 1 {
                        return Err(PolygonError::InvalidTriangulation.into());
                    }
                }
                if same != 1 || opposite != 1 {
                    return Err(PolygonError::InvalidTriangulation.into());
                }
                if a > b {
                    continue; // Count each opposite interior pair once.
                }
            }
            edges = edges
                .checked_add(1)
                .ok_or(PolygonError::InvalidTriangulation)?;
        }
    }
    for corner in scratch {
        control().map_err(TriangulationError::Control)?;
        if corner.next != 1 {
            return Err(PolygonError::InvalidTriangulation.into());
        }
    }
    Ok(edges)
}

fn validate_disk_connectivity<C>(
    scratch: &mut [CornerScratch],
    triangle_count: usize,
    control: &mut impl FnMut() -> Result<(), C>,
) -> Result<(), TriangulationError<C>> {
    // Queue entries occupy the unused ear-link field in the same n scratch slots.
    // Each triangle is enqueued once; scanning its possible neighbors costs O(n).
    scratch[0].previous = 0;
    scratch[0].active = true;
    let mut head = 0;
    let mut tail = 1;
    while head < tail {
        control().map_err(TriangulationError::Control)?;
        let triangle = scratch[scratch[head].previous].triangle;
        head += 1;
        for other in 0..triangle_count {
            control().map_err(TriangulationError::Control)?;
            if scratch[other].active {
                continue;
            }
            let neighbor = scratch[other].triangle;
            let adjacent = triangle_edges(triangle).iter().any(|&[a, b]| {
                triangle_edges(neighbor)
                    .iter()
                    .any(|&[c, d]| a == d && b == c)
            });
            if adjacent {
                scratch[other].active = true;
                scratch[tail].previous = other;
                tail += 1;
            }
        }
    }
    if tail != triangle_count {
        return Err(PolygonError::InvalidTriangulation.into());
    }
    Ok(())
}

fn triangle_edges([a, b, c]: [usize; 3]) -> [[usize; 2]; 3] {
    [[a, b], [b, c], [c, a]]
}

fn ring_next(index: usize, count: usize) -> usize {
    if index == count - 1 { 0 } else { index + 1 }
}

fn edges_improperly_intersect(
    [a, b]: [usize; 2],
    [c, d]: [usize; 2],
    scratch: &[CornerScratch],
) -> bool {
    if (a == c && b == d) || (a == d && b == c) {
        return false; // Incidence separately requires opposite directions exactly once.
    }
    let shared = if a == c {
        Some((a, b, d))
    } else if a == d {
        Some((a, b, c))
    } else if b == c {
        Some((b, a, d))
    } else if b == d {
        Some((b, a, c))
    } else {
        None
    };
    if let Some((origin, first, second)) = shared {
        let origin = scratch[origin].point;
        let first = scratch[first].point;
        let second = scratch[second].point;
        return orient(origin, first, second).abs() <= DETERMINANT_TOLERANCE
            && dot3(subtract3(first, origin), subtract3(second, origin)) >= 0.0;
    }
    segments_may_intersect(
        scratch[a].point,
        scratch[b].point,
        scratch[c].point,
        scratch[d].point,
    )
}

fn point_may_be_in_triangle(
    point: [f64; 3],
    [a, b, c]: [usize; 3],
    scratch: &[CornerScratch],
    winding: f64,
) -> bool {
    winding * orient(scratch[a].point, scratch[b].point, point) >= -DETERMINANT_TOLERANCE
        && winding * orient(scratch[b].point, scratch[c].point, point) >= -DETERMINANT_TOLERANCE
        && winding * orient(scratch[c].point, scratch[a].point, point) >= -DETERMINANT_TOLERANCE
}

fn corner_turn(index: usize, scratch: &[CornerScratch]) -> f64 {
    let corner = &scratch[index];
    orient(
        scratch[corner.previous].point,
        corner.point,
        scratch[corner.next].point,
    )
}

fn orient(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn segments_may_intersect(a: [f64; 3], b: [f64; 3], c: [f64; 3], d: [f64; 3]) -> bool {
    for axis in [0, 1] {
        if a[axis].max(b[axis]) + DISTANCE_TOLERANCE < c[axis].min(d[axis])
            || c[axis].max(d[axis]) + DISTANCE_TOLERANCE < a[axis].min(b[axis])
        {
            return false;
        }
    }
    // For overlapping boxes, accept separation only with a confident same-side
    // predicate. Treating uncertain determinants as ordinary collinearity plus an
    // endpoint test could miss a crossing between two very short segments.
    !same_side(orient(a, b, c), orient(a, b, d)) && !same_side(orient(c, d, a), orient(c, d, b))
}

fn same_side(first: f64, second: f64) -> bool {
    (first > DETERMINANT_TOLERANCE && second > DETERMINANT_TOLERANCE)
        || (first < -DETERMINANT_TOLERANCE && second < -DETERMINANT_TOLERANCE)
}

fn subtract3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[derive(Default)]
struct AreaSum {
    sum: f64,
    correction: f64,
}

impl AreaSum {
    fn add(&mut self, value: f64) {
        let next = self.sum + value;
        self.correction += if self.sum.abs() >= value.abs() {
            (self.sum - next) + value
        } else {
            (value - next) + self.sum
        };
        self.sum = next;
    }

    fn total(&self) -> f64 {
        self.sum + self.correction
    }
}

#[cfg(test)]
#[path = "triangulation/postflight_tests.rs"]
mod postflight_tests;

#[cfg(test)]
#[path = "triangulation/semantic_tests.rs"]
mod semantic_tests;

#[cfg(test)]
#[path = "triangulation/cancellation_tests.rs"]
mod cancellation_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const CONCAVE: [[f64; 3]; 5] = [
        [0.0, 0.0, 0.0],
        [3.0, 0.0, 0.0],
        [3.0, 3.0, 0.0],
        [1.5, 1.0, 0.0],
        [0.0, 3.0, 0.0],
    ];
    const CORNERS: [usize; 5] = [17, 4, 90, 2, 31];

    fn run(positions: &[[f64; 3]], corners: &[usize]) -> Vec<[usize; 3]> {
        let mut scratch = vec![CornerScratch::default(); positions.len()];
        let mut output = vec![[usize::MAX; 3]; positions.len() - 2];
        let count = triangulate_polygon(positions, corners, &mut scratch, &mut output, &mut || {
            Ok::<_, ()>(())
        })
        .expect("valid simple ring");
        assert_eq!(count, positions.len() - 2);
        output
    }

    fn verify(positions: &[[f64; 3]], corners: &[usize], triangles: &[[usize; 3]]) {
        let mut polygon_normal = [0.0; 3];
        for index in 1..positions.len() - 1 {
            let normal = cross3(
                subtract3(positions[index], positions[0]),
                subtract3(positions[index + 1], positions[0]),
            );
            polygon_normal = std::array::from_fn(|axis| polygon_normal[axis] + normal[axis]);
        }
        let mut triangle_normal = [0.0; 3];
        let mut edges = BTreeMap::<(usize, usize), usize>::new();
        let mut used = vec![false; corners.len()];
        for triangle in triangles {
            let [a, b, c] = triangle.map(|corner| {
                let index = corners.iter().position(|&value| value == corner).unwrap();
                used[index] = true;
                index
            });
            assert!(a != b && b != c && c != a);
            let normal = cross3(
                subtract3(positions[b], positions[a]),
                subtract3(positions[c], positions[a]),
            );
            assert!(dot3(normal, polygon_normal) > 0.0, "source winding");
            triangle_normal = std::array::from_fn(|axis| triangle_normal[axis] + normal[axis]);
            for edge in [(a, b), (b, c), (c, a)] {
                *edges.entry(edge).or_default() += 1;
            }
        }
        for axis in 0..3 {
            assert!((triangle_normal[axis] - polygon_normal[axis]).abs() < 1.0e-10);
        }
        assert!(
            used.into_iter().all(|used| used),
            "all source corners retained"
        );
        for index in 0..corners.len() {
            let next = (index + 1) % corners.len();
            assert_eq!(edges.remove(&(index, next)), Some(1), "directed boundary");
            assert!(!edges.contains_key(&(next, index)), "no reversed boundary");
        }
        for (&(a, b), &count) in &edges {
            assert_eq!(count, 1);
            assert_eq!(edges.get(&(b, a)), Some(&1), "opposite interior pair");
        }
        assert_eq!(
            corners.len() + triangles.len(),
            corners.len() + edges.len() / 2 + 1
        );
    }

    #[test]
    fn concave_area_winding_and_source_corner_mapping() {
        verify(
            &CONCAVE[..3],
            &CORNERS[..3],
            &run(&CONCAVE[..3], &CORNERS[..3]),
        );
        let triangles = run(&CONCAVE, &CORNERS);
        verify(&CONCAVE, &CORNERS, &triangles);
        assert_eq!(triangles, run(&CONCAVE, &CORNERS), "deterministic");
        let mut reversed_positions = CONCAVE;
        let mut reversed_corners = CORNERS;
        reversed_positions.reverse();
        reversed_corners.reverse();
        verify(
            &reversed_positions,
            &reversed_corners,
            &run(&reversed_positions, &reversed_corners),
        );
    }

    #[test]
    fn arbitrary_3d_plane_and_dominant_projection_axes() {
        for positions in [
            CONCAVE.map(|[x, y, _]| [7.0, x, y]),
            CONCAVE.map(|[x, y, _]| [x, 7.0, y]),
            CONCAVE.map(|[x, y, _]| [x + 5.0, y - 4.0, 2.0 * x - 3.0 * y + 8.0]),
        ] {
            verify(&positions, &CORNERS, &run(&positions, &CORNERS));
        }
    }

    #[test]
    fn collinear_boundary_corners_are_retained() {
        let positions = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [3.0, 3.0, 0.0],
            [0.0, 3.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        let corners = [9, 8, 7, 6, 5, 4, 3];
        verify(&positions, &corners, &run(&positions, &corners));
        let reversed = positions.into_iter().rev().collect::<Vec<_>>();
        verify(&reversed, &corners, &run(&reversed, &corners));

        let concave_chain = [
            [0.0, 0.0, 0.0],
            [4.0, 0.0, 0.0],
            [4.0, 4.0, 0.0],
            [3.0, 4.0, 0.0],
            [3.0, 1.0, 0.0],
            [2.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 4.0, 0.0],
            [0.0, 4.0, 0.0],
        ];
        let corners = [0, 1, 2, 3, 4, 5, 6, 7, 8];
        verify(&concave_chain, &corners, &run(&concave_chain, &corners));
    }

    #[test]
    fn multiple_reflex_corners_update_neighbor_ears() {
        // Eberly's section 2 example exercises successive reflex-to-convex changes.
        let positions = [
            [3.0, 48.0, 0.0],
            [52.0, 8.0, 0.0],
            [99.0, 50.0, 0.0],
            [138.0, 25.0, 0.0],
            [175.0, 77.0, 0.0],
            [131.0, 72.0, 0.0],
            [111.0, 113.0, 0.0],
            [72.0, 43.0, 0.0],
            [26.0, 55.0, 0.0],
            [29.0, 100.0, 0.0],
        ];
        let corners = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        verify(&positions, &corners, &run(&positions, &corners));
        let reversed = positions.into_iter().rev().collect::<Vec<_>>();
        verify(&reversed, &corners, &run(&reversed, &corners));
    }

    #[test]
    fn normalization_avoids_overflow_and_preserves_small_offsets() {
        let square = [
            [-1.0, -1.0, 0.0],
            [1.0, -1.0, 0.0],
            [1.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0],
        ];
        let corners = [0, 1, 2, 3];
        let expected = run(&square, &corners);
        for scale in [1.0e-310, 1.0e-200, 1.0e200, f64::MAX] {
            let positions = square.map(|point| point.map(|value| value * scale));
            assert_eq!(run(&positions, &corners), expected);
        }
        let translated = square.map(|[x, y, z]| [1.0e15 + x, -1.0e15 + y, z]);
        assert_eq!(run(&translated, &corners), expected);
    }

    fn reject(positions: &[[f64; 3]], corners: &[usize]) -> PolygonError {
        let mut scratch = vec![CornerScratch::default(); positions.len()];
        let mut output = vec![[usize::MAX; 3]; positions.len().saturating_sub(2)];
        match triangulate_polygon(positions, corners, &mut scratch, &mut output, &mut || {
            Ok::<_, ()>(())
        }) {
            Err(TriangulationError::Polygon(error)) => error,
            other => panic!("expected geometry rejection, got {other:?}"),
        }
    }

    #[test]
    fn malformed_and_unsupported_rings_are_rejected() {
        assert_eq!(
            reject(&CONCAVE[..2], &CORNERS[..2]),
            PolygonError::TooFewCorners
        );
        assert_eq!(
            reject(&CONCAVE, &CORNERS[..4]),
            PolygonError::CornerCountMismatch
        );
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut positions = CONCAVE;
            positions[2][1] = value;
            assert_eq!(
                reject(&positions, &CORNERS),
                PolygonError::NonFinite { corner: 2 }
            );
        }
        let mut repeated = CORNERS;
        repeated[4] = repeated[0];
        assert!(matches!(
            reject(&CONCAVE, &repeated),
            PolygonError::RepeatedSourceCorner { .. }
        ));
        let mut positions = CONCAVE;
        positions[4] = positions[0];
        assert!(matches!(
            reject(&positions, &CORNERS),
            PolygonError::CoincidentCorners { .. } | PolygonError::NonSimple { .. }
        ));
        let collinear = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
        assert_eq!(reject(&collinear, &[0, 1, 2]), PolygonError::Degenerate);
        let mut positions = CONCAVE;
        positions[3][2] = 1.0e-4;
        assert!(matches!(
            reject(&positions, &CORNERS),
            PolygonError::NonPlanar { .. }
        ));
    }

    #[test]
    fn crossing_touching_and_overlapping_boundaries_are_rejected() {
        for positions in [
            vec![
                [0.0, 0.0, 0.0],
                [3.0, 3.0, 0.0],
                [0.0, 3.0, 0.0],
                [3.0, 0.0, 0.0],
            ],
            vec![
                [0.0, 0.0, 0.0],
                [3.0, 0.0, 0.0],
                [3.0, 3.0, 0.0],
                [1.5, 0.0, 0.0],
                [0.0, 3.0, 0.0],
            ],
            vec![
                [0.0, 0.0, 0.0],
                [3.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 3.0, 0.0],
                [0.0, 3.0, 0.0],
            ],
        ] {
            let corners: Vec<_> = (0..positions.len()).collect();
            assert!(matches!(
                reject(&positions, &corners),
                PolygonError::NonSimple { .. }
            ));
        }
        assert!(segments_may_intersect(
            [0.0, 0.0, 0.0],
            [1.0e-10, 0.0, 0.0],
            [0.5e-10, -1.0e-4, 0.0],
            [0.5e-10, 1.0e-4, 0.0],
        ));
    }

    #[test]
    fn insufficient_capacity_does_not_touch_output() {
        let sentinel = [usize::MAX; 3];
        let mut output = [sentinel; 4];
        let mut short_scratch = [CornerScratch::default(); 4];
        let mut control = || Ok::<_, ()>(());
        assert_eq!(
            triangulate_polygon(
                &CONCAVE,
                &CORNERS,
                &mut short_scratch,
                &mut output,
                &mut control
            ),
            Err(TriangulationError::Polygon(
                PolygonError::InsufficientScratch
            )),
        );
        assert_eq!(output, [sentinel; 4]);
        let mut scratch = [CornerScratch::default(); 6];
        assert_eq!(
            triangulate_polygon(
                &CONCAVE,
                &CORNERS,
                &mut scratch,
                &mut output[..2],
                &mut control
            ),
            Err(TriangulationError::Polygon(
                PolygonError::InsufficientOutput
            )),
        );
        assert_eq!(output, [sentinel; 4]);
        triangulate_polygon(&CONCAVE, &CORNERS, &mut scratch, &mut output, &mut control).unwrap();
        assert_eq!(output[3], sentinel);
        assert!(!scratch[5].active);
    }

    #[test]
    fn every_control_abort_withholds_a_successful_candidate() {
        let mut scratch = [CornerScratch::default(); 5];
        let mut output = [[usize::MAX; 3]; 3];
        let mut checkpoints = 0;
        triangulate_polygon(&CONCAVE, &CORNERS, &mut scratch, &mut output, &mut || {
            checkpoints += 1;
            Ok::<_, usize>(())
        })
        .unwrap();
        let positions = CONCAVE;
        let corners = CORNERS;
        for stop in 0..checkpoints {
            let mut visited = 0;
            let result =
                triangulate_polygon(&positions, &corners, &mut scratch, &mut output, &mut || {
                    let current = visited;
                    visited += 1;
                    if current == stop { Err(stop) } else { Ok(()) }
                });
            assert_eq!(result, Err(TriangulationError::Control(stop)));
            assert_eq!(visited, stop + 1);
            assert_eq!(positions, CONCAVE);
            assert_eq!(corners, CORNERS);
            // There is no candidate count on failure. The transaction must discard
            // output, even when the abort follows its final source-identity write.
        }
    }
}
