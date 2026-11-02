//! Camera and punctual-light placement from the evaluator's single world table.

use crate::deformation::{determinant, dot};
use crate::material::uniform_scalar;
use crate::{Error, Result, cap, f32_at, vec_with_capacity};
use baozi::{CameraProjection, LightKindCategory};
use baozi_asset_consumer::{ApplicationAsset, EvaluatedAsset, EvaluationMatrix};

pub const IDENTITY: EvaluationMatrix = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

#[derive(Clone, Copy, Debug)]
pub enum CameraInput {
    /// Node placement, not the possibly shared definition's index.
    Node(usize),
    Explicit {
        projection: CameraProjection,
        world: EvaluationMatrix,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct CameraFrame {
    pub view_projection: EvaluationMatrix,
    pub position: [f64; 3],
    pub reflected: bool,
    pub orthographic: bool,
    /// Direction from every surface toward an orthographic viewer, with no position dependence.
    pub view_direction: [f64; 3],
}

pub fn multiply(a: EvaluationMatrix, b: EvaluationMatrix) -> EvaluationMatrix {
    std::array::from_fn(|c| std::array::from_fn(|r| (0..4).map(|k| a[k][r] * b[c][k]).sum()))
}
pub fn project(m: EvaluationMatrix, p: [f64; 4]) -> [f64; 4] {
    std::array::from_fn(|r| (0..4).map(|c| m[c][r] * p[c]).sum())
}

fn rigid(
    world: EvaluationMatrix,
    basis: baozi::Mat4,
    node: Option<usize>,
    entity: &'static str,
) -> Result<EvaluationMatrix> {
    let frame = multiply(world, basis.cols.map(|c| c.map(f64::from)));
    let fail = || Error::NonRigidSpatialPose { node, entity };
    if frame.iter().flatten().any(|v| !v.is_finite())
        || frame[0][3] != 0.0
        || frame[1][3] != 0.0
        || frame[2][3] != 0.0
        || frame[3][3] != 1.0
    {
        return Err(fail());
    }
    for i in 0..3 {
        for j in 0..3 {
            let a = [frame[i][0], frame[i][1], frame[i][2]];
            let b = [frame[j][0], frame[j][1], frame[j][2]];
            if (dot(a, b) - if i == j { 1.0 } else { 0.0 }).abs() > 1e-6 {
                return Err(fail());
            }
        }
    }
    if (determinant(frame).abs() - 1.0).abs() > 1e-6 {
        return Err(fail());
    }
    Ok(frame)
}

pub fn camera(
    asset: &ApplicationAsset,
    evaluation: &EvaluatedAsset,
    input: CameraInput,
    width: u32,
    height: u32,
) -> Result<CameraFrame> {
    if width == 0 || height == 0 {
        return Err(Error::InvalidLimit("empty viewport"));
    }
    let (projection, world, node) = match input {
        CameraInput::Node(node) => {
            let definition = asset
                .nodes
                .get(node)
                .and_then(|n| n.camera)
                .and_then(|i| asset.cameras.get(i))
                .ok_or(Error::InvalidAsset("camera placement node"))?;
            (
                definition.projection,
                *evaluation
                    .world_transforms
                    .get(node)
                    .ok_or(Error::InvalidAsset("camera world transform"))?,
                Some(node),
            )
        }
        CameraInput::Explicit { projection, world } => (projection, world, None),
    };
    let placement = rigid(world, projection.axes().basis_matrix(), node, "camera")?;
    let mut view = IDENTITY;
    for c in 0..3 {
        for r in 0..3 {
            view[c][r] = placement[r][c];
        }
    }
    let position = [placement[3][0], placement[3][1], placement[3][2]];
    let translation: [f64; 3] =
        std::array::from_fn(|r| -(0..3).map(|c| view[c][r] * position[c]).sum::<f64>());
    view[3][..3].copy_from_slice(&translation);
    let mut p = [[0.0; 4]; 4];
    match projection {
        CameraProjection::Perspective(v) => {
            let near = f64::from(v.near());
            let f = 1.0 / (f64::from(v.vertical_fov_radians()) * 0.5).tan();
            let aspect = v
                .aspect_ratio()
                .map(f64::from)
                .unwrap_or(f64::from(width) / f64::from(height));
            p[0][0] = f / aspect;
            p[1][1] = f;
            p[2][3] = -1.0;
            if let Some(far) = v.far() {
                let far = f64::from(far);
                p[2][2] = far / (near - far);
                p[3][2] = far * near / (near - far);
            } else {
                p[2][2] = -1.0;
                p[3][2] = -near;
            }
        }
        CameraProjection::Orthographic(v) => {
            let near = f64::from(v.near());
            let far = f64::from(v.far());
            p[0][0] = 1.0 / f64::from(v.horizontal_magnification());
            p[1][1] = 1.0 / f64::from(v.vertical_magnification());
            p[2][2] = 1.0 / (near - far);
            p[3][2] = near / (near - far);
            p[3][3] = 1.0;
        }
    }
    let view_projection = multiply(p, view);
    if view_projection.iter().flatten().any(|v| !v.is_finite()) {
        return Err(Error::InvalidAsset("nonfinite camera projection"));
    }
    Ok(CameraFrame {
        view_projection,
        position,
        reflected: determinant(placement) < 0.0,
        orthographic: matches!(projection, CameraProjection::Orthographic(_)),
        view_direction: [placement[2][0], placement[2][1], placement[2][2]],
    })
}

#[derive(Debug, Clone, Copy)]
pub struct LightFrame {
    pub position: [f64; 3],
    /// World direction from the emitter toward surfaces; local reference -Z.
    pub outgoing: [f64; 3],
    pub kind: LightKindCategory,
    pub tint: [f64; 3],
    pub intensity: f64,
    pub range: Option<f64>,
    pub cos_inner: f64,
    pub cos_outer: f64,
    pub meters_per_unit: f64,
}

impl LightFrame {
    pub fn bytes(self) -> Result<[u8; 64]> {
        if self.tint.iter().any(|v| !v.is_finite() || *v < 0.0)
            || !self.intensity.is_finite()
            || self.intensity < 0.0
            || self.outgoing.iter().any(|v| !v.is_finite())
            || (dot(self.outgoing, self.outgoing) - 1.0).abs() > 1e-6
            || self.range.is_some_and(|v| !v.is_finite() || v <= 0.0)
            || !self.meters_per_unit.is_finite()
            || self.meters_per_unit <= 0.0
            || !(-1.0..=1.0).contains(&self.cos_inner)
            || !(-1.0..=1.0).contains(&self.cos_outer)
            || self.cos_inner < self.cos_outer
        {
            return Err(Error::InvalidAsset("invalid explicit punctual light"));
        }
        let mut out = [0; 64];
        let kind = match self.kind {
            LightKindCategory::Directional => 0.0,
            LightKindCategory::Point => 1.0,
            LightKindCategory::Spot => 2.0,
        };
        for (i, v) in [
            self.position[0],
            self.position[1],
            self.position[2],
            self.range.unwrap_or(0.0),
            self.outgoing[0],
            self.outgoing[1],
            self.outgoing[2],
            kind,
            self.tint[0],
            self.tint[1],
            self.tint[2],
            self.intensity,
            self.cos_inner,
            self.cos_outer,
            f64::from(u32::from(self.range.is_some())),
            self.meters_per_unit,
        ]
        .into_iter()
        .enumerate()
        {
            f32_at(&mut out, i * 4, uniform_scalar(v, i)?);
        }
        Ok(out)
    }
}

pub fn lights(
    asset: &ApplicationAsset,
    evaluation: &EvaluatedAsset,
    max_lights: u32,
) -> Result<Vec<LightFrame>> {
    let mut out = vec_with_capacity(
        asset
            .nodes
            .iter()
            .filter(|n| n.light.is_some())
            .count()
            .min(max_lights as usize),
    )?;
    for (node, n) in asset.nodes.iter().enumerate() {
        let Some(index) = n.light else {
            continue;
        };
        cap(
            "active light placements",
            out.len() as u64 + 1,
            u64::from(max_lights),
        )?;
        let light = asset
            .lights
            .get(index)
            .ok_or(Error::InvalidAsset("light index"))?
            .definition;
        let world = *evaluation
            .world_transforms
            .get(node)
            .ok_or(Error::InvalidAsset("light world matrix"))?;
        let placement = rigid(world, light.axes().basis_matrix(), Some(node), "light")?;
        let kind = light.kind();
        let tint = light.tint();
        let meters = if kind.category() == LightKindCategory::Directional {
            1.0
        } else {
            asset
                .space
                .unit_scale_to_meters
                .filter(|v| v.is_finite() && *v > 0.0)
                .ok_or(Error::UnknownSceneUnitScale)?
        };
        out.push(LightFrame {
            position: [placement[3][0], placement[3][1], placement[3][2]],
            outgoing: [-placement[2][0], -placement[2][1], -placement[2][2]],
            kind: kind.category(),
            tint: [f64::from(tint.x), f64::from(tint.y), f64::from(tint.z)],
            intensity: f64::from(light.intensity()),
            range: kind.range().map(f64::from),
            cos_inner: kind
                .inner_cone_radians()
                .map_or(1.0, |v| f64::from(v).cos()),
            cos_outer: kind
                .outer_cone_radians()
                .map_or(1.0, |v| f64::from(v).cos()),
            meters_per_unit: meters,
        });
    }
    Ok(out)
}

pub fn range_attenuation(distance: f64, range: Option<f64>) -> f64 {
    range.map_or(1.0, |range| {
        (1.0 - (distance / range).powi(4)).clamp(0.0, 1.0).powi(2)
    })
}
pub fn spot_attenuation(cosine: f64, inner: f64, outer: f64) -> f64 {
    if inner == outer {
        if cosine >= outer { 1.0 } else { 0.0 }
    } else {
        ((cosine - outer) / (inner - outer)).clamp(0.0, 1.0).powi(2)
    }
}
