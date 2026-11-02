//! Renderer frame seam; clip sampling and skinning belong to the asset evaluator.

use crate::device::Planning;
use crate::{Error, Result, vec_with_capacity};
use baozi_asset_consumer::{DrawBuffer, EvaluatedAsset, EvaluatedDraw, EvaluationMatrix};

/// F64 world frames in the source draw's complete vertex order, including unused vertices.
/// Orientation is +1/-1 for the complete deformation's linear determinant, per vertex.
/// Tangent W is the resulting world-frame handedness, not the original source sign.
#[derive(Clone, Copy)]
pub struct DrawFrames<'a> {
    pub instance: usize,
    pub draw: usize,
    pub normals: &'a [[f64; 3]],
    pub tangents: Option<&'a [[f64; 4]]>,
    pub orientations: &'a [i8],
}

/// Borrow the evaluator's explicit world frames without copying or evaluating them again.
pub fn evaluated_frames(evaluation: &EvaluatedAsset) -> Result<Vec<DrawFrames<'_>>> {
    let mut frames = vec_with_capacity(evaluation.draws.len())?;
    for draw in &evaluation.draws {
        match (&draw.world_normals, &draw.world_orientations) {
            (Some(normals), Some(orientations)) => frames.push(DrawFrames {
                instance: draw.instance,
                draw: draw.draw,
                normals,
                tangents: draw.world_tangents.as_deref(),
                orientations,
            }),
            (None, _) => {}
            _ => return Err(Error::InvalidAsset("evaluated normal/orientation presence")),
        }
    }
    Ok(frames)
}

pub(crate) struct WorldFrame {
    pub normal: [f64; 3],
    pub tangent: Option<[f64; 4]>,
    pub orientation: i8,
}

pub(crate) struct FrameTable<'a> {
    normals: Option<&'a [[f64; 3]]>,
    tangents: Option<&'a [[f64; 4]]>,
    orientations: &'a [i8],
}

impl FrameTable<'_> {
    pub(crate) fn at(&self, vertex: usize) -> WorldFrame {
        WorldFrame {
            normal: self.normals.map_or([0.0, 0.0, 1.0], |n| n[vertex]),
            tangent: self.tangents.map(|t| t[vertex]),
            orientation: self.orientations[vertex],
        }
    }
    pub(crate) fn has_tangents(&self) -> bool {
        self.tangents.is_some()
    }
}

pub(crate) fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(crate) fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(crate) fn determinant(m: EvaluationMatrix) -> f64 {
    dot(
        [m[0][0], m[0][1], m[0][2]],
        cross([m[1][0], m[1][1], m[1][2]], [m[2][0], m[2][1], m[2][2]]),
    )
}

/// Checks the seam without manufacturing bind-pose normals for any active deformation.
pub(crate) fn frames_for_draw<'a>(
    source: &DrawBuffer,
    evaluated: &'a EvaluatedDraw,
    supplied: &[DrawFrames<'a>],
    lit: bool,
    planning: &mut Planning<'_>,
) -> Result<FrameTable<'a>> {
    let instance = evaluated.instance;
    let draw = evaluated.draw;
    let mut supplied_frame = None;
    for f in supplied {
        planning.charge(1)?;
        if f.instance == instance && f.draw == draw {
            if supplied_frame.is_some() {
                return Err(Error::InvalidAsset("duplicate explicit frames"));
            }
            supplied_frame = Some(*f);
        }
    }
    // Unlit still retains evaluated Normal/Tangent streams when present. Only absent fields
    // receive shader placeholders; this does not manufacture a static frame transformation.
    let explicit = supplied_frame.or_else(|| {
        if !lit {
            evaluated
                .world_normals
                .as_deref()
                .zip(evaluated.world_orientations.as_deref())
                .map(|(normals, orientations)| DrawFrames {
                    instance,
                    draw,
                    normals,
                    tangents: evaluated.world_tangents.as_deref(),
                    orientations,
                })
        } else {
            None
        }
    });
    if explicit.is_none() && lit {
        return Err(Error::ExplicitFramesRequired { instance, draw });
    }
    if let Some(f) = explicit
        && (f.normals.len() != source.vertices.len()
            || f.orientations.len() != source.vertices.len()
            || f.tangents.is_some_and(|t| t.len() != source.vertices.len()))
    {
        return Err(Error::InvalidAsset(
            "frame table does not cover every source vertex",
        ));
    }
    let table = FrameTable {
        normals: explicit.map(|f| f.normals),
        tangents: explicit.and_then(|f| f.tangents),
        orientations: explicit
            .map(|f| f.orientations)
            .or(evaluated.world_orientations.as_deref())
            .ok_or(Error::ExplicitFramesRequired { instance, draw })?,
    };
    if table.orientations.len() != source.vertices.len() {
        return Err(Error::InvalidAsset(
            "frame table does not cover every source vertex",
        ));
    }
    for (vertex, v) in source.vertices.iter().enumerate() {
        planning.charge(16)?;
        let failure = || Error::InvalidFrame {
            instance,
            draw,
            vertex,
            reason: "singular, nonfinite or degenerate world frame",
        };
        if let Some(f) = explicit {
            let n = f.normals[vertex];
            if n.iter().any(|v| !v.is_finite()) || (dot(n, n) - 1.0).abs() > 1e-6 {
                return Err(failure());
            }
            let orientation = f.orientations[vertex];
            if ![-1, 1].contains(&orientation) {
                return Err(failure());
            }
            if let Some(t) = f.tangents {
                let t = t[vertex];
                if ![-1.0, 1.0].contains(&t[3]) {
                    return Err(failure());
                }
                let raw = [t[0], t[1], t[2]];
                if raw.iter().any(|v| !v.is_finite())
                    || (dot(raw, raw) - 1.0).abs() > 1e-6
                    || dot(n, raw).abs() > 1e-6
                {
                    return Err(failure());
                }
            }
        } else {
            // Unlit does not use a normal, but culling still needs deformation orientation.
            let orientation = table.orientations[vertex];
            if ![-1, 1].contains(&orientation) {
                return Err(failure());
            }
        }
        if lit && v.normal.is_none() {
            return Err(failure());
        }
    }
    Ok(table)
}
