//! A deterministic F64 position and frozen-weight frame reference over detached buffers.
//!
//! This module owns no Scene, generation, clock, renderer, or shading state. Morph deltas
//! precede skinning; influences and morph weights are never clamped or renormalized.

use std::{collections::TryReserveError, error::Error, fmt};

use baozi::{
    AnimationExtrapolation, AnimationInterpolation, AnimationTimeDomain, CancellationToken,
    MorphDeltaSemantic, SkinMatrixSpaces, TopologyKind,
};

use crate::buffers::{ApplicationAsset, Channel, ChannelTarget, Clip, DrawBuffer, TangentEncoding};

/// Column-major matrices acting on column vectors, without F32 intermediate rounding.
pub type EvaluationMatrix = [[f64; 4]; 4];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SampleTime {
    /// A time in the selected clip's declared units, including unknown-rate ticks.
    Declared(f64),
    /// Converts seconds using the clip's declared positive frame/tick rate.
    Seconds(f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramePolicy {
    /// Evaluate positions only. Success makes no claim about deformed shading.
    PositionOnly,
    /// Authored morph shading deltas followed by the frozen-weight linear-blend frame.
    FrozenWeight,
    /// Evaluate deformed normals and orientation only; do not request authored tangents.
    /// Suitable for untextured lighting that does not consume a tangent frame.
    FrozenWeightNormalsOnly,
    /// Evaluate deformed normals and orientation, rejecting active nonzero tangent morphs.
    /// The renderer owns construction and validation of each binding's UV derivative frame;
    /// unused source tangent vectors are neither evaluated nor certified by this policy.
    TriangleUvDerivative,
}

#[derive(Debug, Clone, Copy)]
pub struct EvaluationLimits {
    pub max_output_vertices: usize,
    pub max_output_draws: usize,
    pub max_palette_matrices: usize,
    pub max_output_bytes: usize,
    /// Cumulative scratch Vec capacity, including allocations released during evaluation.
    pub max_scratch_bytes: usize,
    /// Sum of output and cumulative scratch capacity bytes; excludes the borrowed input.
    pub max_total_bytes: usize,
    pub max_work_units: u64,
}

impl Default for EvaluationLimits {
    fn default() -> Self {
        Self {
            max_output_vertices: 2_000_000,
            max_output_draws: 4_096,
            max_palette_matrices: 4_096,
            max_output_bytes: 512 * 1024 * 1024,
            max_scratch_bytes: 128 * 1024 * 1024,
            max_total_bytes: 512 * 1024 * 1024,
            max_work_units: 200_000_000,
        }
    }
}

/// The frame policy is mandatory; there is no implicit bind-pose shading fallback.
pub struct EvaluationRequest<'a> {
    pub frames: FramePolicy,
    pub limits: EvaluationLimits,
    pub cancellation: Option<&'a CancellationToken>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EvaluationAccounting {
    pub output_vertices: usize,
    pub output_draws: usize,
    pub output_bytes: usize,
    pub scratch_bytes: usize,
    pub work_units: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvaluationLimit {
    OutputVertices,
    OutputDraws,
    PaletteMatrices,
    OutputBytes,
    ScratchBytes,
    TotalBytes,
    Work,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameFailure {
    SingularLinearMap,
    DegenerateNormal,
    DegenerateTangent,
    MissingNormal,
    UnsupportedTangentEncoding,
    InvalidHandedness,
    MissingMorphShadingDeltas,
    UnsupportedTangentMorphNormalMap,
    MixedFrameOrientation,
}

#[derive(Debug)]
pub enum EvaluationError {
    Allocation(TryReserveError),
    Cancelled,
    LimitExceeded {
        limit: EvaluationLimit,
    },
    SizeOverflow,
    Frame {
        instance: usize,
        draw: usize,
        vertex: usize,
        reason: FrameFailure,
    },
    InvalidAsset(&'static str),
    InvalidClip {
        clip: usize,
    },
    NonFiniteTime,
    UnknownTimeRate,
    InvalidTimeRate,
    InvalidHierarchy,
    InvalidChannel {
        channel: usize,
        reason: &'static str,
    },
    UnsupportedExtrapolation {
        channel: usize,
        mode: AnimationExtrapolation,
    },
    MatrixAuthoredAnimation {
        node: usize,
    },
    ZeroLengthQuaternion {
        channel: usize,
    },
    UnsupportedSkinSpaces {
        skin: usize,
    },
    UnsupportedSkinCorrection {
        skin: usize,
    },
    /// The linear world transform is singular or numerically rank deficient in F64.
    SingularInstanceTransform {
        instance: usize,
    },
    NumericFailure {
        operation: &'static str,
    },
}

impl fmt::Display for EvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Allocation(error) => write!(formatter, "evaluation allocation failed: {error}"),
            Self::Cancelled => formatter.write_str("evaluation cancelled"),
            Self::LimitExceeded { limit } => {
                write!(formatter, "evaluation limit exceeded: {limit:?}")
            }
            Self::SizeOverflow => formatter.write_str("evaluation accounting size overflow"),
            Self::Frame {
                instance,
                draw,
                vertex,
                reason,
            } => write!(
                formatter,
                "instance {instance} draw {draw} vertex {vertex}: {reason:?}"
            ),
            Self::InvalidAsset(reason) => write!(formatter, "invalid owned asset: {reason}"),
            Self::InvalidClip { clip } => {
                write!(formatter, "animation clip {clip} is out of range")
            }
            Self::NonFiniteTime => formatter.write_str("sample time must be finite"),
            Self::UnknownTimeRate => {
                formatter.write_str("seconds conversion requires a known tick rate")
            }
            Self::InvalidTimeRate => {
                formatter.write_str("frame/tick rate must be finite and positive")
            }
            Self::InvalidHierarchy => {
                formatter.write_str("owned nodes must form the declared rooted forest")
            }
            Self::InvalidChannel { channel, reason } => {
                write!(formatter, "invalid channel {channel}: {reason}")
            }
            Self::UnsupportedExtrapolation { channel, mode } => write!(
                formatter,
                "channel {channel} uses unsupported {mode:?} extrapolation"
            ),
            Self::MatrixAuthoredAnimation { node } => write!(
                formatter,
                "TRS animation targets matrix-authored node {node}"
            ),
            Self::ZeroLengthQuaternion { channel } => write!(
                formatter,
                "channel {channel} evaluates to a zero-length quaternion"
            ),
            Self::UnsupportedSkinSpaces { skin } => write!(
                formatter,
                "skin {skin} does not use canonical matrix spaces"
            ),
            Self::UnsupportedSkinCorrection { skin } => write!(
                formatter,
                "skin {skin} declares an unsupported source correction"
            ),
            Self::SingularInstanceTransform { instance } => write!(
                formatter,
                "skin instance {instance} has a singular or numerically rank-deficient world transform"
            ),
            Self::NumericFailure { operation } => {
                write!(formatter, "reference numeric failure during {operation}")
            }
        }
    }
}

impl Error for EvaluationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Allocation(error) => Some(error),
            _ => None,
        }
    }
}

impl From<TryReserveError> for EvaluationError {
    fn from(error: TryReserveError) -> Self {
        Self::Allocation(error)
    }
}

type Result<T> = std::result::Result<T, EvaluationError>;

#[derive(Debug)]
pub struct EvaluatedDraw {
    /// Dense application indices into the input asset; these are not Scene identities.
    pub instance: usize,
    pub draw: usize,
    /// Morphed and, when bound, skinned instance-local XYZ positions in draw vertex order.
    pub positions: Vec<[f64; 3]>,
    /// World XYZ from the full weighted homogeneous skin equation, without division by W.
    pub world_positions: Vec<[f64; 3]>,
    /// Normalized inverse-transpose normals; absent source fields remain absent.
    pub world_normals: Option<Vec<[f64; 3]>>,
    /// Orthogonalized tangents with reflected SignInW handedness, only for FrozenWeight.
    pub world_tangents: Option<Vec<[f64; 4]>>,
    /// Sign of the frozen-weight world linear map, in draw vertex order.
    pub world_orientations: Option<Vec<i8>>,
}

#[derive(Debug)]
pub struct EvaluatedAsset {
    pub local_transforms: Vec<EvaluationMatrix>,
    pub world_transforms: Vec<EvaluationMatrix>,
    /// Animation, then optional instance override, then mesh defaults; one vector per instance.
    pub morph_weights: Vec<Vec<f64>>,
    pub draws: Vec<EvaluatedDraw>,
    pub accounting: EvaluationAccounting,
}

/// Samples a single clip without looping or clamping its clock, or the authored pose for `None`.
///
/// Only Step, Linear and CubicSpline curves and canonical skins without correction are supported.
/// Linear/Repeat channel extrapolation is rejected even when the requested time is in range.
/// SourceDefault leaves the immutable authored component/default weights in place. Constant
/// uses an endpoint. A matrix-authored node is never decomposed to accept TRS animation.
/// Singular or numerically rank-deficient instance transforms return a named failure. An inverse
/// whose two-sided residual cannot meet the F64 reference accuracy limit is a numeric failure.
///
/// FrozenWeight applies authored Position/Normal/Tangent morph deltas before skinning. Its
/// per-vertex linear map uses every influence in original row order, without normalization.
/// This frozen-weight approximation does not differentiate spatial weight gradients. Active
/// position morphs require Normal delta coverage when source normals exist. Missing or
/// degenerate shading data fails the entire requested evaluation; no bind-pose fallback exists.
/// Projected unit tangent directions with maximum component at most 64 * F64 epsilon are
/// numerically degenerate. Reflections change tangent handedness, not source normal bits.
/// FrozenWeightNormalsOnly and TriangleUvDerivative explicitly omit authored tangent output
/// and validation; they certify deformed normals/orientation only. FrozenWeight still rejects
/// degenerate authored tangents. TriangleUvDerivative additionally rejects active nonzero
/// tangent morphs whose appearance cannot be represented by the renderer's derivative frame.
///
/// Requested Vec capacity bytes are admitted before fallible allocation, then reconciled against
/// actual Vec capacity before population. Accounting includes both position arrays, both transform
/// tables, nested Vec headers and all scratch reservations, cumulatively even after release.
/// It excludes input storage, stack, allocator overhead and allocator/OS internals, and is not
/// an RSS or peak-live-memory claim. Work counts table/scalar visits and fixed-size numeric
/// kernels, including validation and the output count pass; it is not an instruction count.
/// Cancellation is polled before/after reservations and each charged visit/kernel. Fixed-size
/// matrix operations (including the checked inverse) are bounded cancellation intervals.
/// No partial output escapes an error. The result retains no borrow of the asset or Scene.
pub fn evaluate(
    asset: &ApplicationAsset,
    clip: Option<usize>,
    time: SampleTime,
    request: EvaluationRequest<'_>,
) -> std::result::Result<EvaluatedAsset, EvaluationError> {
    let mut budget = Budget::new(&request);
    budget.poll()?;
    let clip = clip
        .map(|index| {
            asset
                .animations
                .get(index)
                .ok_or(EvaluationError::InvalidClip { clip: index })
        })
        .transpose()?;
    let time = declared_time(clip, time)?;
    if asset.mesh_morph_weights.len() != asset.mesh_count {
        return Err(EvaluationError::InvalidAsset(
            "mesh default-weight table length",
        ));
    }
    for draw in &asset.draws {
        budget.work(1)?;
        if draw.mesh >= asset.mesh_count {
            return Err(EvaluationError::InvalidAsset("draw mesh is out of range"));
        }
    }
    // Reserve the outer output once; never grow it implicitly during instance expansion.
    for instance in &asset.instances {
        budget.work(1)?;
        for draw in &asset.draws {
            budget.work(1)?;
            if draw.mesh == instance.mesh {
                budget.output_draw(draw.vertices.len())?;
            }
        }
    }
    let mut draws = budget.reserved(budget.accounting.output_draws, Storage::Output)?;
    let mut poses = budget.reserved(asset.nodes.len(), Storage::Scratch)?;
    for node in &asset.nodes {
        budget.work(1)?;
        let pose = if let Some(trs) = node.transform.authored_trs() {
            let t = trs.translation();
            let q = trs.rotation();
            let s = trs.scale();
            LocalPose::Trs {
                translation: [t.x, t.y, t.z].map(f64::from),
                rotation: [q.x, q.y, q.z, q.w].map(f64::from),
                scale: [s.x, s.y, s.z].map(f64::from),
            }
        } else {
            LocalPose::Matrix(
                node.transform
                    .local_matrix()
                    .cols
                    .map(|col| col.map(f64::from)),
            )
        };
        poses.push(pose);
    }
    let mut morph_weights = budget.reserved(asset.instances.len(), Storage::Output)?;
    for instance in &asset.instances {
        budget.work(1)?;
        if instance.node >= asset.nodes.len() {
            return Err(EvaluationError::InvalidAsset(
                "instance node is out of range",
            ));
        }
        let defaults =
            asset
                .mesh_morph_weights
                .get(instance.mesh)
                .ok_or(EvaluationError::InvalidAsset(
                    "instance mesh is out of range",
                ))?;
        for value in defaults {
            budget.work(1)?;
            if !value.is_finite() {
                return Err(EvaluationError::InvalidAsset("nonfinite mesh morph weight"));
            }
        }
        let weights = instance.morph_weights.as_deref().unwrap_or(defaults);
        if weights.len() != defaults.len() {
            return Err(EvaluationError::InvalidAsset(
                "invalid instance morph override",
            ));
        }
        let mut copied = budget.reserved(weights.len(), Storage::Output)?;
        for &weight in weights {
            budget.work(1)?;
            if !weight.is_finite() {
                return Err(EvaluationError::InvalidAsset(
                    "invalid instance morph override",
                ));
            }
            copied.push(f64::from(weight));
        }
        morph_weights.push(copied);
    }
    if let Some(clip) = clip {
        apply_channels(clip, time, &mut poses, &mut morph_weights, &mut budget)?;
    }
    let mut local_transforms = budget.reserved(poses.len(), Storage::Output)?;
    for pose in poses {
        budget.work(1)?;
        let matrix = pose.matrix();
        finite(matrix.into_iter().flatten(), "local transform composition")?;
        local_transforms.push(matrix);
    }
    let world_transforms = world_transforms(asset, &local_transforms, &mut budget)?;
    for (instance_index, instance) in asset.instances.iter().enumerate() {
        budget.work(1)?;
        let world = world_transforms[instance.node];
        let skin = instance
            .skin
            .map(|skin_index| {
                let skin = asset
                    .skins
                    .get(skin_index)
                    .ok_or(EvaluationError::InvalidAsset(
                        "instance skin is out of range",
                    ))?;
                if skin.matrix_spaces.correction().is_some() {
                    return Err(EvaluationError::UnsupportedSkinCorrection { skin: skin_index });
                }
                if skin.matrix_spaces != SkinMatrixSpaces::canonical() {
                    return Err(EvaluationError::UnsupportedSkinSpaces { skin: skin_index });
                }
                if skin.joints.len() != skin.inverse_binds.len() {
                    return Err(EvaluationError::InvalidAsset(
                        "skin palette/bind length mismatch",
                    ));
                }
                budget.check(
                    skin.joints.len(),
                    request.limits.max_palette_matrices,
                    EvaluationLimit::PaletteMatrices,
                )?;
                budget.work(1)?;
                let inverse = inverse_affine(world, instance_index)?;
                let mut palette = budget.reserved(skin.joints.len(), Storage::Scratch)?;
                for (&joint, bind) in skin.joints.iter().zip(&skin.inverse_binds) {
                    budget.work(1)?;
                    let joint_world = *world_transforms
                        .get(joint)
                        .ok_or(EvaluationError::InvalidAsset("skin joint is out of range"))?;
                    if !bind.is_finite() || !bind.is_affine() {
                        return Err(EvaluationError::InvalidAsset("invalid inverse bind matrix"));
                    }
                    let matrix = multiply(joint_world, bind.cols.map(|col| col.map(f64::from)));
                    finite(matrix.into_iter().flatten(), "skin palette composition")?;
                    palette.push(matrix);
                }
                Ok((inverse, palette))
            })
            .transpose()?;
        for (draw_index, draw) in asset.draws.iter().enumerate() {
            budget.work(1)?;
            if draw.mesh != instance.mesh {
                continue;
            }
            let weights = &morph_weights[instance_index];
            validate_deformation(
                draw,
                weights.len(),
                skin.as_ref().map(|(_, palette)| palette.len()),
                &mut budget,
            )?;
            let frames = request.frames != FramePolicy::PositionOnly;
            let (has_normals, has_tangents) = if frames {
                validate_frames(
                    draw,
                    weights,
                    request.frames,
                    instance_index,
                    draw_index,
                    &mut budget,
                )?
            } else {
                (false, false)
            };
            let mut positions = budget.reserved(draw.vertices.len(), Storage::Output)?;
            let mut world_positions = budget.reserved(draw.vertices.len(), Storage::Output)?;
            let mut world_normals = has_normals
                .then(|| budget.reserved(draw.vertices.len(), Storage::Output))
                .transpose()?;
            let mut world_tangents = has_tangents
                .then(|| budget.reserved(draw.vertices.len(), Storage::Output))
                .transpose()?;
            let mut world_orientations = frames
                .then(|| budget.reserved(draw.vertices.len(), Storage::Output))
                .transpose()?;
            for (vertex_index, vertex) in draw.vertices.iter().enumerate() {
                budget.work(1)?;
                let mut position = vertex.position;
                let mut normal = vertex.normal;
                let mut tangent = if has_tangents { vertex.tangent } else { None };
                for stream in &draw.morphs {
                    budget.work(1)?;
                    let output: Option<&mut [f64]> = match stream.semantic {
                        MorphDeltaSemantic::Position => Some(&mut position),
                        MorphDeltaSemantic::Normal if frames => normal.as_mut().map(|v| &mut v[..]),
                        MorphDeltaSemantic::Tangent if frames => {
                            tangent.as_mut().map(|v| &mut v[..3])
                        }
                        _ => None,
                    };
                    if let Some(output) = output {
                        for (value, delta) in output.iter_mut().zip(stream.values[vertex_index]) {
                            *value += weights[stream.target] * f64::from(delta);
                        }
                    }
                }
                finite(position, "morph evaluation")?;
                let point = [position[0], position[1], position[2], 1.0];
                let mut linear = if skin.is_some() { [[0.0; 4]; 4] } else { world };
                let (local, world_point) = if let Some((inverse, palette)) = &skin {
                    let rows = draw
                        .influences
                        .as_ref()
                        .ok_or(EvaluationError::InvalidAsset(
                            "skinned draw has no influences",
                        ))?;
                    let mut blended = [0.0; 4];
                    let range = rows.offsets[vertex_index]..rows.offsets[vertex_index + 1];
                    for influence in range {
                        budget.work(1)?;
                        let joint = usize::try_from(rows.joints[influence])
                            .map_err(|_| EvaluationError::InvalidAsset("joint index overflow"))?;
                        let transformed = transform(palette[joint], point);
                        for (value, component) in blended.iter_mut().zip(transformed) {
                            *value += f64::from(rows.weights[influence]) * component;
                        }
                        if frames {
                            for (out, column) in linear[..3].iter_mut().zip(&palette[joint][..3]) {
                                for (out, component) in out[..3].iter_mut().zip(&column[..3]) {
                                    *out += f64::from(rows.weights[influence]) * component;
                                }
                            }
                        }
                    }
                    // Retain weighted W through inverse(M). Replacing it with one would
                    // silently change the meaning of influence rows whose sum is not one.
                    (transform(*inverse, blended), blended)
                } else {
                    (point, transform(world, point))
                };
                finite(local, "instance-local skin evaluation")?;
                finite(world_point, "world position evaluation")?;
                positions.push([local[0], local[1], local[2]]);
                world_positions.push([world_point[0], world_point[1], world_point[2]]);
                if frames {
                    budget.work(1)?;
                    let location = FrameLocation {
                        instance: instance_index,
                        draw: draw_index,
                        vertex: vertex_index,
                    };
                    let EvaluatedFrame {
                        normal,
                        tangent,
                        orientation,
                    } = evaluate_frame(linear, normal, tangent, location)?;
                    if let (Some(output), Some(value)) = (&mut world_normals, normal) {
                        output.push(value);
                    }
                    if let (Some(output), Some(value)) = (&mut world_tangents, tangent) {
                        output.push(value);
                    }
                    if let Some(output) = &mut world_orientations {
                        output.push(orientation);
                    }
                }
            }
            if let Some(orientations) = &world_orientations {
                validate_orientation(draw, orientations, instance_index, draw_index, &mut budget)?;
            }
            draws.push(EvaluatedDraw {
                instance: instance_index,
                draw: draw_index,
                positions,
                world_positions,
                world_normals,
                world_tangents,
                world_orientations,
            });
        }
    }
    budget.poll()?;
    Ok(EvaluatedAsset {
        local_transforms,
        world_transforms,
        morph_weights,
        draws,
        accounting: budget.accounting,
    })
}

fn declared_time(clip: Option<&Clip>, time: SampleTime) -> Result<f64> {
    let (SampleTime::Declared(value) | SampleTime::Seconds(value)) = time;
    if !value.is_finite() {
        return Err(EvaluationError::NonFiniteTime);
    }
    let Some(clip) = clip else { return Ok(value) };
    if !clip.duration.is_finite() || clip.duration < 0.0 {
        return Err(EvaluationError::InvalidAsset("invalid clip duration"));
    }
    let rate = match clip.time_domain {
        AnimationTimeDomain::Seconds => Some(1.0),
        AnimationTimeDomain::Frames { rate } => Some(rate),
        AnimationTimeDomain::Ticks { rate } => rate,
    };
    if rate.is_some_and(|rate| !rate.is_finite() || rate <= 0.0) {
        return Err(EvaluationError::InvalidTimeRate);
    }
    let value = match time {
        SampleTime::Declared(value) => value,
        SampleTime::Seconds(value) => value * rate.ok_or(EvaluationError::UnknownTimeRate)?,
    };
    finite([value], "seconds conversion")?;
    Ok(value)
}

enum LocalPose {
    Matrix(EvaluationMatrix),
    Trs {
        translation: [f64; 3],
        rotation: [f64; 4],
        scale: [f64; 3],
    },
}

impl LocalPose {
    fn matrix(self) -> EvaluationMatrix {
        match self {
            Self::Matrix(matrix) => matrix,
            Self::Trs {
                translation: [tx, ty, tz],
                rotation: [x, y, z, w],
                scale: [sx, sy, sz],
            } => [
                [
                    (1.0 - 2.0 * (y * y + z * z)) * sx,
                    2.0 * (x * y + w * z) * sx,
                    2.0 * (x * z - w * y) * sx,
                    0.0,
                ],
                [
                    2.0 * (x * y - w * z) * sy,
                    (1.0 - 2.0 * (x * x + z * z)) * sy,
                    2.0 * (y * z + w * x) * sy,
                    0.0,
                ],
                [
                    2.0 * (x * z + w * y) * sz,
                    2.0 * (y * z - w * x) * sz,
                    (1.0 - 2.0 * (x * x + y * y)) * sz,
                    0.0,
                ],
                [tx, ty, tz, 1.0],
            ],
        }
    }
}

fn apply_channels(
    clip: &Clip,
    time: f64,
    poses: &mut [LocalPose],
    weights: &mut [Vec<f64>],
    budget: &mut Budget<'_>,
) -> Result<()> {
    let mut node_targets = budget.filled(poses.len(), 0_u8, Storage::Scratch)?;
    let mut morph_targets = budget.filled(weights.len(), false, Storage::Scratch)?;
    for (index, channel) in clip.channels.iter().enumerate() {
        budget.work(1)?;
        let invalid = |reason| EvaluationError::InvalidChannel {
            channel: index,
            reason,
        };
        let output: &mut [f64] = match channel.target {
            ChannelTarget::MorphWeights(instance) => {
                let seen = morph_targets
                    .get_mut(instance)
                    .ok_or_else(|| invalid("instance target is out of range"))?;
                if *seen {
                    return Err(invalid("duplicate morph target"));
                }
                *seen = true;
                weights
                    .get_mut(instance)
                    .ok_or_else(|| invalid("instance target is out of range"))?
            }
            target => {
                let (node, bit) = match target {
                    ChannelTarget::Translation(node) => (node, 1),
                    ChannelTarget::Rotation(node) => (node, 2),
                    ChannelTarget::Scale(node) => (node, 4),
                    ChannelTarget::MorphWeights(_) => return Err(invalid("invalid node target")),
                };
                let seen = node_targets
                    .get_mut(node)
                    .ok_or_else(|| invalid("node target is out of range"))?;
                if *seen & bit != 0 {
                    return Err(invalid("duplicate node component target"));
                }
                *seen |= bit;
                let LocalPose::Trs {
                    translation,
                    rotation,
                    scale,
                } = &mut poses[node]
                else {
                    return Err(EvaluationError::MatrixAuthoredAnimation { node });
                };
                match target {
                    ChannelTarget::Translation(_) => translation,
                    ChannelTarget::Rotation(_) => rotation,
                    ChannelTarget::Scale(_) => scale,
                    ChannelTarget::MorphWeights(_) => return Err(invalid("invalid node target")),
                }
            }
        };
        validate_channel(channel, output.len(), index, budget)?;
        sample_channel(channel, time, output, index, budget)?;
    }
    Ok(())
}

fn validate_channel(
    channel: &Channel,
    width: usize,
    index: usize,
    budget: &mut Budget<'_>,
) -> Result<()> {
    let invalid = |reason| EvaluationError::InvalidChannel {
        channel: index,
        reason,
    };
    if width == 0 || channel.width != width || channel.times.is_empty() {
        return Err(invalid("invalid width or empty key table"));
    }
    let count = width
        .checked_mul(channel.times.len())
        .ok_or_else(|| invalid("key table size overflow"))?;
    if channel.values.len() != count {
        return Err(invalid("invalid key value table"));
    }
    for value in &channel.values {
        budget.work(1)?;
        if !value.is_finite() {
            return Err(invalid("invalid key value table"));
        }
    }
    let mut previous = None;
    for &time in &channel.times {
        budget.work(1)?;
        if !time.is_finite() || previous.is_some_and(|previous| previous >= time) {
            return Err(invalid("key times must be finite and strictly increasing"));
        }
        previous = Some(time);
    }
    match channel.interpolation {
        AnimationInterpolation::CubicSpline => {
            for tangents in [&channel.in_tangents, &channel.out_tangents] {
                let Some(values) = tangents.as_ref().filter(|values| values.len() == count) else {
                    return Err(invalid("cubic derivatives must match the key table"));
                };
                for value in values {
                    budget.work(1)?;
                    if !value.is_finite() {
                        return Err(invalid("cubic derivatives must match the key table"));
                    }
                }
            }
        }
        _ if channel.in_tangents.is_some() || channel.out_tangents.is_some() => {
            return Err(invalid("non-cubic channel has derivatives"));
        }
        _ => {}
    }
    for mode in [channel.pre_extrapolation, channel.post_extrapolation] {
        if matches!(
            mode,
            AnimationExtrapolation::Linear | AnimationExtrapolation::Repeat
        ) {
            return Err(EvaluationError::UnsupportedExtrapolation {
                channel: index,
                mode,
            });
        }
    }
    Ok(())
}

fn sample_channel(
    channel: &Channel,
    time: f64,
    output: &mut [f64],
    index: usize,
    budget: &mut Budget<'_>,
) -> Result<()> {
    budget.work(1)?;
    let last = channel.times.len() - 1;
    let endpoint = if time < channel.times[0] {
        Some((0, channel.pre_extrapolation))
    } else if time > channel.times[last] {
        Some((last, channel.post_extrapolation))
    } else {
        None
    };
    let key = if let Some((key, mode)) = endpoint {
        if mode == AnimationExtrapolation::SourceDefault {
            return Ok(());
        }
        Some(key)
    } else {
        let lower = channel.times.partition_point(|key| *key <= time) - 1;
        if channel.interpolation == AnimationInterpolation::Step || channel.times[lower] == time {
            Some(lower)
        } else {
            None
        }
    };
    if let Some(key) = key {
        let start = key * channel.width;
        for (out, value) in output
            .iter_mut()
            .zip(&channel.values[start..start + channel.width])
        {
            budget.work(1)?;
            *out = f64::from(*value);
        }
    } else {
        let lower = channel.times.partition_point(|key| *key <= time) - 1;
        let upper = lower + 1;
        let interval = channel.times[upper] - channel.times[lower];
        let u = (time - channel.times[lower]) / interval;
        finite([interval, u], "key interval arithmetic")?;
        let a = lower * channel.width;
        let b = upper * channel.width;
        if channel.interpolation == AnimationInterpolation::Linear {
            if matches!(channel.target, ChannelTarget::Rotation(_)) {
                let first = std::array::from_fn(|lane| f64::from(channel.values[a + lane]));
                let second = std::array::from_fn(|lane| f64::from(channel.values[b + lane]));
                output.copy_from_slice(&slerp(first, second, u, index)?);
            } else {
                for (lane, out) in output.iter_mut().enumerate() {
                    budget.work(1)?;
                    *out = (1.0 - u) * f64::from(channel.values[a + lane])
                        + u * f64::from(channel.values[b + lane]);
                }
            }
        } else {
            let incoming = channel
                .in_tangents
                .as_ref()
                .ok_or(EvaluationError::InvalidChannel {
                    channel: index,
                    reason: "missing incoming derivatives",
                })?;
            let outgoing =
                channel
                    .out_tangents
                    .as_ref()
                    .ok_or(EvaluationError::InvalidChannel {
                        channel: index,
                        reason: "missing outgoing derivatives",
                    })?;
            let u2 = u * u;
            let u3 = u2 * u;
            let [h00, h10, h01, h11] = [
                2.0 * u3 - 3.0 * u2 + 1.0,
                u3 - 2.0 * u2 + u,
                -2.0 * u3 + 3.0 * u2,
                u3 - u2,
            ];
            for (lane, out) in output.iter_mut().enumerate() {
                budget.work(1)?;
                // Tangents are raw component derivatives in declared time units, including
                // quaternion derivatives. Never normalize or sign-adjust cubic inputs.
                *out = h00 * f64::from(channel.values[a + lane])
                    + h10 * interval * f64::from(outgoing[a + lane])
                    + h01 * f64::from(channel.values[b + lane])
                    + h11 * interval * f64::from(incoming[b + lane]);
            }
        }
    }
    for &value in output.iter() {
        budget.work(1)?;
        finite([value], "curve evaluation")?;
    }
    if matches!(channel.target, ChannelTarget::Rotation(_)) {
        normalize_quaternion(output, index)?;
    }
    Ok(())
}

fn normalize_quaternion(value: &mut [f64], channel: usize) -> Result<()> {
    finite(value.iter().copied(), "quaternion normalization")?;
    let scale = value
        .iter()
        .fold(0.0_f64, |scale, value| scale.max(value.abs()));
    if scale == 0.0 {
        return Err(EvaluationError::ZeroLengthQuaternion { channel });
    }
    let length = value
        .iter()
        .map(|value| (value / scale).powi(2))
        .sum::<f64>()
        .sqrt();
    for value in value {
        *value = (*value / scale) / length;
    }
    Ok(())
}

fn slerp(mut first: [f64; 4], mut second: [f64; 4], u: f64, channel: usize) -> Result<[f64; 4]> {
    normalize_quaternion(&mut first, channel)?;
    normalize_quaternion(&mut second, channel)?;
    let mut dot = first.iter().zip(second).map(|(a, b)| a * b).sum::<f64>();
    if dot < 0.0 {
        second = second.map(|value| -value);
        dot = -dot;
    }
    let mut result = if dot >= 1.0 {
        std::array::from_fn(|lane| (1.0 - u) * first[lane] + u * second[lane])
    } else {
        let angle = dot.clamp(-1.0, 1.0).acos();
        let denominator = angle.sin();
        let a = ((1.0 - u) * angle).sin() / denominator;
        let b = (u * angle).sin() / denominator;
        std::array::from_fn(|lane| a * first[lane] + b * second[lane])
    };
    normalize_quaternion(&mut result, channel)?;
    Ok(result)
}

fn world_transforms(
    asset: &ApplicationAsset,
    local: &[EvaluationMatrix],
    budget: &mut Budget<'_>,
) -> Result<Vec<EvaluationMatrix>> {
    let mut parents = budget.filled(local.len(), None, Storage::Scratch)?;
    for (index, node) in asset.nodes.iter().enumerate() {
        budget.work(1)?;
        for &child in &node.children {
            budget.work(1)?;
            let parent = parents
                .get_mut(child)
                .ok_or(EvaluationError::InvalidHierarchy)?;
            if parent.replace(index).is_some() {
                return Err(EvaluationError::InvalidHierarchy);
            }
        }
    }
    let mut world = budget.filled(local.len(), IDENTITY, Storage::Output)?;
    let mut seen = budget.filled(local.len(), false, Storage::Scratch)?;
    let mut queue = budget.reserved(local.len(), Storage::Scratch)?;
    for &root in &asset.roots {
        budget.work(1)?;
        let visited = seen
            .get_mut(root)
            .ok_or(EvaluationError::InvalidHierarchy)?;
        if *visited || parents[root].is_some() {
            return Err(EvaluationError::InvalidHierarchy);
        }
        *visited = true;
        world[root] = local[root];
        queue.push(root);
    }
    let mut cursor = 0;
    while let Some(&parent) = queue.get(cursor) {
        budget.work(1)?;
        for &child in &asset.nodes[parent].children {
            budget.work(1)?;
            if seen[child] {
                return Err(EvaluationError::InvalidHierarchy);
            }
            seen[child] = true;
            world[child] = multiply(world[parent], local[child]);
            finite(
                world[child].into_iter().flatten(),
                "parent/world composition",
            )?;
            queue.push(child);
        }
        cursor += 1;
    }
    if queue.len() != local.len() {
        return Err(EvaluationError::InvalidHierarchy);
    }
    Ok(world)
}

fn validate_deformation(
    draw: &DrawBuffer,
    width: usize,
    palette: Option<usize>,
    budget: &mut Budget<'_>,
) -> Result<()> {
    for stream in &draw.morphs {
        budget.work(1)?;
        if stream.target >= width || stream.values.len() != draw.vertices.len() {
            return Err(EvaluationError::InvalidAsset("invalid morph stream"));
        }
        for delta in &stream.values {
            budget.work(1)?;
            if !delta.iter().all(|value| value.is_finite()) {
                return Err(EvaluationError::InvalidAsset("invalid morph stream"));
            }
        }
    }
    if let Some(palette) = palette {
        let rows = draw
            .influences
            .as_ref()
            .ok_or(EvaluationError::InvalidAsset(
                "skinned draw has no influences",
            ))?;
        let offsets = draw
            .vertices
            .len()
            .checked_add(1)
            .ok_or(EvaluationError::InvalidAsset(
                "influence row count overflow",
            ))?;
        if rows.offsets.len() != offsets
            || rows.joints.len() != rows.weights.len()
            || rows.offsets.first() != Some(&0)
            || rows.offsets.last() != Some(&rows.joints.len())
        {
            return Err(EvaluationError::InvalidAsset("invalid skin influence rows"));
        }
        for pair in rows.offsets.windows(2) {
            budget.work(1)?;
            if pair[0] > pair[1] {
                return Err(EvaluationError::InvalidAsset("invalid skin influence rows"));
            }
        }
        for (&joint, weight) in rows.joints.iter().zip(&rows.weights) {
            budget.work(1)?;
            if !weight.is_finite() || usize::try_from(joint).map_or(true, |joint| joint >= palette)
            {
                return Err(EvaluationError::InvalidAsset("invalid skin influence rows"));
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct FrameLocation {
    instance: usize,
    draw: usize,
    vertex: usize,
}

impl FrameLocation {
    fn error(self, reason: FrameFailure) -> EvaluationError {
        EvaluationError::Frame {
            instance: self.instance,
            draw: self.draw,
            vertex: self.vertex,
            reason,
        }
    }
}

fn validate_frames(
    draw: &DrawBuffer,
    weights: &[f64],
    policy: FramePolicy,
    instance: usize,
    draw_index: usize,
    budget: &mut Budget<'_>,
) -> Result<(bool, bool)> {
    let normals = draw
        .vertices
        .first()
        .is_some_and(|vertex| vertex.normal.is_some());
    let authored_tangents = policy == FramePolicy::FrozenWeight;
    let tangents = authored_tangents
        && draw
            .vertices
            .first()
            .is_some_and(|vertex| vertex.tangent.is_some());
    for (vertex_index, vertex) in draw.vertices.iter().enumerate() {
        budget.work(1)?;
        let location = FrameLocation {
            instance,
            draw: draw_index,
            vertex: vertex_index,
        };
        if vertex.normal.is_some() != normals
            || (authored_tangents && vertex.tangent.is_some() != tangents)
        {
            return Err(EvaluationError::InvalidAsset(
                "inconsistent frame field presence",
            ));
        }
        if let Some(tangent) = vertex.tangent.filter(|_| authored_tangents) {
            if !normals {
                return Err(location.error(FrameFailure::MissingNormal));
            }
            if !draw
                .tangent_conventions
                .as_ref()
                .is_some_and(|conventions| conventions.encoding == TangentEncoding::SignInW)
            {
                return Err(location.error(FrameFailure::UnsupportedTangentEncoding));
            }
            if tangent[3] != 1.0 && tangent[3] != -1.0 {
                return Err(location.error(FrameFailure::InvalidHandedness));
            }
        }
    }
    let mut normal_coverage = budget.filled(weights.len(), false, Storage::Scratch)?;
    for stream in &draw.morphs {
        budget.work(1)?;
        if stream.semantic == MorphDeltaSemantic::Normal {
            normal_coverage[stream.target] = true;
        }
    }
    for stream in &draw.morphs {
        budget.work(1)?;
        if weights[stream.target] == 0.0 {
            continue;
        }
        let missing_normals = normals
            && stream.semantic == MorphDeltaSemantic::Position
            && !normal_coverage[stream.target];
        let tangent_conflict = policy == FramePolicy::TriangleUvDerivative
            && stream.semantic == MorphDeltaSemantic::Tangent;
        if missing_normals || tangent_conflict {
            for (vertex, delta) in stream.values.iter().enumerate() {
                budget.work(1)?;
                if delta.iter().any(|value| *value != 0.0) {
                    let location = FrameLocation {
                        instance,
                        draw: draw_index,
                        vertex,
                    };
                    let reason = if missing_normals {
                        FrameFailure::MissingMorphShadingDeltas
                    } else {
                        FrameFailure::UnsupportedTangentMorphNormalMap
                    };
                    return Err(location.error(reason));
                }
            }
        }
    }
    Ok((normals, tangents))
}

struct EvaluatedFrame {
    normal: Option<[f64; 3]>,
    tangent: Option<[f64; 4]>,
    orientation: i8,
}

fn evaluate_frame(
    linear: EvaluationMatrix,
    normal: Option<[f64; 3]>,
    tangent: Option<[f64; 4]>,
    location: FrameLocation,
) -> Result<EvaluatedFrame> {
    // Embed only the linear map: weighted skin W and translation belong to positions.
    let mut affine = IDENTITY;
    for (out, column) in affine[..3].iter_mut().zip(&linear[..3]) {
        out[..3].copy_from_slice(&column[..3]);
    }
    finite(affine.into_iter().flatten(), "frame linear map")?;
    let inverse = inverse_affine(affine, location.instance).map_err(|error| match error {
        EvaluationError::SingularInstanceTransform { .. } => {
            location.error(FrameFailure::SingularLinearMap)
        }
        error => error,
    })?;
    let scale = affine[..3]
        .iter()
        .flat_map(|column| &column[..3])
        .fold(0.0_f64, |scale, value| scale.max(value.abs()));
    let [a, b, c] = std::array::from_fn::<_, 3, _>(|col| {
        std::array::from_fn::<_, 3, _>(|row| affine[col][row] / scale)
    });
    let determinant = dot(a, cross(b, c));
    finite([determinant], "frame orientation")?;
    if determinant == 0.0 {
        return Err(location.error(FrameFailure::SingularLinearMap));
    }
    let orientation = if determinant < 0.0 { -1 } else { 1 };
    let normal = normal
        .map(|normal| {
            let transformed =
                std::array::from_fn(|row| (0..3).map(|col| inverse[row][col] * normal[col]).sum());
            unit(transformed, location, FrameFailure::DegenerateNormal)
        })
        .transpose()?;
    let tangent = tangent
        .map(|tangent| -> Result<[f64; 4]> {
            let normal = normal.ok_or_else(|| location.error(FrameFailure::MissingNormal))?;
            let raw = transform(affine, [tangent[0], tangent[1], tangent[2], 0.0]);
            let direction = unit(
                [raw[0], raw[1], raw[2]],
                location,
                FrameFailure::DegenerateTangent,
            )?;
            let projection = dot(normal, direction);
            let projected = std::array::from_fn(|lane| direction[lane] - projection * normal[lane]);
            if projected
                .iter()
                .all(|value| value.abs() <= 64.0 * f64::EPSILON)
            {
                return Err(location.error(FrameFailure::DegenerateTangent));
            }
            let tangent_xyz = unit(projected, location, FrameFailure::DegenerateTangent)?;
            Ok([
                tangent_xyz[0],
                tangent_xyz[1],
                tangent_xyz[2],
                tangent[3] * f64::from(orientation),
            ])
        })
        .transpose()?;
    Ok(EvaluatedFrame {
        normal,
        tangent,
        orientation,
    })
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(value: [f64; 3], location: FrameLocation, reason: FrameFailure) -> Result<[f64; 3]> {
    finite(value, "frame normalization")?;
    let scale = value
        .into_iter()
        .fold(0.0_f64, |scale, value| scale.max(value.abs()));
    if scale == 0.0 {
        return Err(location.error(reason));
    }
    let scaled = value.map(|value| value / scale);
    let length = dot(scaled, scaled).sqrt();
    Ok(scaled.map(|value| value / length))
}

fn validate_orientation(
    draw: &DrawBuffer,
    orientations: &[i8],
    instance: usize,
    draw_index: usize,
    budget: &mut Budget<'_>,
) -> Result<()> {
    let width = match draw.topology {
        TopologyKind::Points => 1,
        TopologyKind::LineList => 2,
        TopologyKind::TriangleList => 3,
        _ => {
            return Err(EvaluationError::InvalidAsset(
                "frame evaluation requires list topology",
            ));
        }
    };
    if !draw.indices.len().is_multiple_of(width) {
        return Err(EvaluationError::InvalidAsset("incomplete indexed element"));
    }
    for element in draw.indices.chunks_exact(width) {
        budget.work(1)?;
        let mut first = None;
        for &index in element {
            let vertex = usize::try_from(index).map_err(|_| EvaluationError::SizeOverflow)?;
            let &orientation = orientations
                .get(vertex)
                .ok_or(EvaluationError::InvalidAsset("draw index is out of range"))?;
            if first.is_some_and(|first| first != orientation) {
                return Err(FrameLocation {
                    instance,
                    draw: draw_index,
                    vertex,
                }
                .error(FrameFailure::MixedFrameOrientation));
            }
            first = Some(orientation);
        }
    }
    Ok(())
}

const IDENTITY: EvaluationMatrix = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

fn transform(matrix: EvaluationMatrix, value: [f64; 4]) -> [f64; 4] {
    std::array::from_fn(|row| (0..4).map(|col| matrix[col][row] * value[col]).sum())
}

fn multiply(left: EvaluationMatrix, right: EvaluationMatrix) -> EvaluationMatrix {
    right.map(|column| transform(left, column))
}

fn inverse_affine(matrix: EvaluationMatrix, instance: usize) -> Result<EvaluationMatrix> {
    // Normalize only the linear part: uniform scale and translation do not determine rank.
    // The roundoff guard is relative to that normalized 3x3 norm, never to scene units.
    const ROUNDOFF: f64 = 64.0 * f64::EPSILON;
    let scale = matrix[..3]
        .iter()
        .flat_map(|column| &column[..3])
        .fold(0.0_f64, |scale, value| scale.max(value.abs()));
    if scale == 0.0 {
        return Err(EvaluationError::SingularInstanceTransform { instance });
    }
    let linear: [[f64; 3]; 3] =
        std::array::from_fn(|row| std::array::from_fn(|col| matrix[col][row] / scale));
    let norm = linear
        .iter()
        .map(|row| row.iter().map(|value| value.abs()).sum::<f64>())
        .fold(0.0_f64, f64::max);
    let mut rows: [[f64; 6]; 3] = std::array::from_fn(|row| {
        std::array::from_fn(|col| {
            if col < 3 {
                linear[row][col]
            } else {
                f64::from(u8::from(col - 3 == row))
            }
        })
    });
    for col in 0..3 {
        let mut pivot = col;
        for row in col + 1..3 {
            if rows[row][col].abs() > rows[pivot][col].abs() {
                pivot = row;
            }
        }
        if rows[pivot][col].abs() <= ROUNDOFF * norm {
            return Err(EvaluationError::SingularInstanceTransform { instance });
        }
        rows.swap(pivot, col);
        let divisor = rows[col][col];
        for value in &mut rows[col] {
            *value /= divisor;
        }
        let pivot_row = rows[col];
        for (row, values) in rows.iter_mut().enumerate() {
            if row != col {
                let factor = values[col];
                for (value, pivot) in values.iter_mut().zip(pivot_row) {
                    *value -= factor * pivot;
                }
            }
        }
        finite(rows.into_iter().flatten(), "instance transform inversion")?;
    }
    let mut result = IDENTITY;
    for (row, values) in rows.iter().enumerate() {
        for col in 0..3 {
            result[col][row] = values[col + 3] / scale;
        }
    }
    finite(result.into_iter().flatten(), "instance inverse rescaling")?;

    // Check the actually stored inverse after rescaling, in dimensionless coordinates.
    let inverse: [[f64; 3]; 3] =
        std::array::from_fn(|row| std::array::from_fn(|col| result[col][row] * scale));
    let inverse_norm = inverse
        .iter()
        .map(|row| row.iter().map(|value| value.abs()).sum::<f64>())
        .fold(0.0_f64, f64::max);
    let reciprocal_condition = (1.0 / norm) / inverse_norm;
    finite(
        [inverse_norm, reciprocal_condition],
        "instance inverse condition estimate",
    )?;
    if reciprocal_condition <= ROUNDOFF {
        return Err(EvaluationError::SingularInstanceTransform { instance });
    }
    // Certify the stored operands, not a second rounded normalization of them. An ordinary
    // cancelling dot product can round an inaccurate inverse's residual all the way to zero.
    let residual_limit = (ROUNDOFF / reciprocal_condition).min(f64::EPSILON.sqrt());
    let original: [[f64; 3]; 3] =
        std::array::from_fn(|row| std::array::from_fn(|col| matrix[col][row]));
    let retained: [[f64; 3]; 3] =
        std::array::from_fn(|row| std::array::from_fn(|col| result[col][row]));
    for (left, right) in [(&original, &retained), (&retained, &original)] {
        let mut residual = 0.0_f64;
        for (row, values) in left.iter().enumerate() {
            let mut row_residual = 0.0_f64;
            for col in [0, 1, 2] {
                let terms = std::array::from_fn::<_, 4, _>(|k| {
                    if k < 3 {
                        values[k] * right[k][col]
                    } else {
                        -f64::from(u8::from(row == col))
                    }
                });
                row_residual = (row_residual + inverse_residual_upper(terms)?).next_up();
            }
            finite([row_residual], "instance inverse residual")?;
            residual = residual.max(row_residual);
        }
        if residual > residual_limit {
            return Err(EvaluationError::NumericFailure {
                operation: "instance inverse residual",
            });
        }
    }
    for row in [0, 1, 2] {
        result[3][row] = -(0..3)
            .map(|col| result[col][row] * matrix[3][col])
            .sum::<f64>();
    }
    finite(result.into_iter().flatten(), "inverse instance translation")?;

    // Scale the complete translation vector in each multiplication order. A row containing
    // only propagated roundoff cannot serve as its own relative denominator.
    for (left, right) in [(matrix, result), (result, matrix)] {
        let terms = std::array::from_fn::<_, 3, _>(|row| {
            std::array::from_fn::<_, 4, _>(|col| left[col][row] * right[3][col])
        });
        finite(
            terms.into_iter().flatten(),
            "inverse affine translation residual",
        )?;
        let magnitude = terms
            .iter()
            .flatten()
            .fold(0.0_f64, |max, value| max.max(value.abs()));
        if magnitude == 0.0 {
            continue;
        }
        let scaled = terms.map(|row| row.map(|value| value / magnitude));
        let denominator = scaled
            .iter()
            .map(|row| row.iter().map(|value| value.abs()).sum::<f64>())
            .fold(0.0_f64, f64::max);
        let mut residual = 0.0_f64;
        for row in scaled {
            residual = residual.max(inverse_residual_upper(row)?);
        }
        if residual > ROUNDOFF * denominator {
            return Err(EvaluationError::NumericFailure {
                operation: "inverse affine translation residual",
            });
        }
    }
    Ok(result)
}

/// A conservative bound for a four-term inverse-check sum, including the products' roundoff.
/// Sixteen epsilons cover four products, three additions and the residual/scale arithmetic;
/// outward positive sums and a subnormal allowance keep rounded zero from certifying accuracy.
fn inverse_residual_upper(terms: [f64; 4]) -> Result<f64> {
    finite(terms, "instance inverse residual")?;
    let magnitude = terms
        .iter()
        .fold(0.0_f64, |sum, value| (sum + value.abs()).next_up());
    let error = ((16.0 * f64::EPSILON * magnitude).next_up() + 16.0 * f64::from_bits(1)).next_up();
    let upper = (terms.into_iter().sum::<f64>().abs() + error).next_up();
    finite([magnitude, upper], "instance inverse residual")?;
    Ok(upper)
}

fn finite(values: impl IntoIterator<Item = f64>, operation: &'static str) -> Result<()> {
    if values.into_iter().any(|value| !value.is_finite()) {
        return Err(EvaluationError::NumericFailure { operation });
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Storage {
    Output,
    Scratch,
}

struct Budget<'a> {
    limits: EvaluationLimits,
    cancellation: Option<&'a CancellationToken>,
    accounting: EvaluationAccounting,
}

impl<'a> Budget<'a> {
    fn new(request: &EvaluationRequest<'a>) -> Self {
        Self {
            limits: request.limits,
            cancellation: request.cancellation,
            accounting: EvaluationAccounting::default(),
        }
    }

    fn poll(&self) -> Result<()> {
        if self
            .cancellation
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Err(EvaluationError::Cancelled);
        }
        Ok(())
    }

    fn work(&mut self, units: u64) -> Result<()> {
        self.poll()?;
        let total = self
            .accounting
            .work_units
            .checked_add(units)
            .ok_or(EvaluationError::SizeOverflow)?;
        if total > self.limits.max_work_units {
            return Err(EvaluationError::LimitExceeded {
                limit: EvaluationLimit::Work,
            });
        }
        self.accounting.work_units = total;
        Ok(())
    }

    fn check(&self, value: usize, maximum: usize, limit: EvaluationLimit) -> Result<()> {
        if value > maximum {
            return Err(EvaluationError::LimitExceeded { limit });
        }
        Ok(())
    }

    fn output_draw(&mut self, vertices: usize) -> Result<()> {
        let draws = self
            .accounting
            .output_draws
            .checked_add(1)
            .ok_or(EvaluationError::SizeOverflow)?;
        let vertices = self
            .accounting
            .output_vertices
            .checked_add(vertices)
            .ok_or(EvaluationError::SizeOverflow)?;
        self.check(
            draws,
            self.limits.max_output_draws,
            EvaluationLimit::OutputDraws,
        )?;
        self.check(
            vertices,
            self.limits.max_output_vertices,
            EvaluationLimit::OutputVertices,
        )?;
        self.accounting.output_draws = draws;
        self.accounting.output_vertices = vertices;
        Ok(())
    }

    fn admit_bytes(&self, bytes: usize, storage: Storage) -> Result<()> {
        let (current, maximum, limit) = match storage {
            Storage::Output => (
                self.accounting.output_bytes,
                self.limits.max_output_bytes,
                EvaluationLimit::OutputBytes,
            ),
            Storage::Scratch => (
                self.accounting.scratch_bytes,
                self.limits.max_scratch_bytes,
                EvaluationLimit::ScratchBytes,
            ),
        };
        let added = current
            .checked_add(bytes)
            .ok_or(EvaluationError::SizeOverflow)?;
        self.check(added, maximum, limit)?;
        let total = self
            .accounting
            .output_bytes
            .checked_add(self.accounting.scratch_bytes)
            .and_then(|total| total.checked_add(bytes))
            .ok_or(EvaluationError::SizeOverflow)?;
        self.check(
            total,
            self.limits.max_total_bytes,
            EvaluationLimit::TotalBytes,
        )
    }

    fn reserved<T>(&mut self, capacity: usize, storage: Storage) -> Result<Vec<T>> {
        self.poll()?;
        let bytes = capacity
            .checked_mul(std::mem::size_of::<T>())
            .ok_or(EvaluationError::SizeOverflow)?;
        self.admit_bytes(bytes, storage)?;
        let mut values = Vec::new();
        values.try_reserve_exact(capacity)?;
        let bytes = values
            .capacity()
            .checked_mul(std::mem::size_of::<T>())
            .ok_or(EvaluationError::SizeOverflow)?;
        self.admit_bytes(bytes, storage)?;
        match storage {
            Storage::Output => self.accounting.output_bytes += bytes,
            Storage::Scratch => self.accounting.scratch_bytes += bytes,
        }
        self.poll()?;
        Ok(values)
    }

    fn filled<T: Copy>(&mut self, len: usize, value: T, storage: Storage) -> Result<Vec<T>> {
        let mut values = self.reserved(len, storage)?;
        for _ in 0..len {
            self.work(1)?;
            values.push(value);
        }
        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_after_admitted_work_stops_the_next_visit_and_reservation() {
        let token = CancellationToken::new();
        let request = EvaluationRequest {
            frames: FramePolicy::PositionOnly,
            limits: EvaluationLimits::default(),
            cancellation: Some(&token),
        };
        let mut budget = Budget::new(&request);
        budget.work(1).unwrap();
        let _scratch = budget.reserved::<u8>(16, Storage::Scratch).unwrap();
        let before = budget.accounting;
        token.cancel();
        assert!(matches!(budget.work(1), Err(EvaluationError::Cancelled)));
        assert!(matches!(
            budget.reserved::<u8>(16, Storage::Output),
            Err(EvaluationError::Cancelled)
        ));
        assert_eq!(budget.accounting, before);
    }

    #[test]
    fn overflow_rejects_unrepresentable_reservations_and_expansion_counts() {
        let request = EvaluationRequest {
            frames: FramePolicy::PositionOnly,
            limits: EvaluationLimits {
                max_output_vertices: usize::MAX,
                max_output_draws: usize::MAX,
                max_output_bytes: usize::MAX,
                max_scratch_bytes: usize::MAX,
                max_total_bytes: usize::MAX,
                ..EvaluationLimits::default()
            },
            cancellation: None,
        };
        let mut budget = Budget::new(&request);
        assert!(matches!(
            budget.reserved::<EvaluationMatrix>(usize::MAX, Storage::Output),
            Err(EvaluationError::SizeOverflow)
        ));
        assert_eq!(budget.accounting.output_bytes, 0);
        budget.output_draw(usize::MAX).unwrap();
        assert!(matches!(
            budget.output_draw(1),
            Err(EvaluationError::SizeOverflow)
        ));
    }
}
