//! A deterministic F64 position-evaluation reference over detached application buffers.
//!
//! This module owns no Scene, generation, clock, renderer, or shading state. Morph deltas
//! precede skinning; influences and morph weights are never clamped or renormalized.

use std::{collections::TryReserveError, error::Error, fmt};

use baozi::{
    AnimationExtrapolation, AnimationInterpolation, AnimationTimeDomain, MorphDeltaSemantic,
    SkinMatrixSpaces,
};

use crate::buffers::{ApplicationAsset, Channel, ChannelTarget, Clip, DrawBuffer};

/// Column-major matrices acting on column vectors, without F32 intermediate rounding.
pub type EvaluationMatrix = [[f64; 4]; 4];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SampleTime {
    /// A time in the selected clip's declared units, including unknown-rate ticks.
    Declared(f64),
    /// Converts seconds using the clip's declared positive frame/tick rate.
    Seconds(f64),
}

#[derive(Debug)]
pub enum EvaluationError {
    Allocation(TryReserveError),
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
}

#[derive(Debug)]
pub struct EvaluatedAsset {
    pub local_transforms: Vec<EvaluationMatrix>,
    pub world_transforms: Vec<EvaluationMatrix>,
    /// Animation, then optional instance override, then mesh defaults; one vector per instance.
    pub morph_weights: Vec<Vec<f64>>,
    pub draws: Vec<EvaluatedDraw>,
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
/// All output and scratch allocation is fallible and independent of import/postprocess budgets.
/// The returned data retains no borrow of the asset or its original Scene/importer/IO.
pub fn evaluate(
    asset: &ApplicationAsset,
    clip: Option<usize>,
    time: SampleTime,
) -> std::result::Result<EvaluatedAsset, EvaluationError> {
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
    let mut poses = reserved(asset.nodes.len())?;
    for node in &asset.nodes {
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
    let mut morph_weights = reserved(asset.instances.len())?;
    for instance in &asset.instances {
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
        if !defaults.iter().all(|value| value.is_finite()) {
            return Err(EvaluationError::InvalidAsset("nonfinite mesh morph weight"));
        }
        let weights = instance.morph_weights.as_deref().unwrap_or(defaults);
        if weights.len() != defaults.len() || !weights.iter().all(|value| value.is_finite()) {
            return Err(EvaluationError::InvalidAsset(
                "invalid instance morph override",
            ));
        }
        let mut copied = reserved(weights.len())?;
        copied.extend(weights.iter().copied().map(f64::from));
        morph_weights.push(copied);
    }
    if let Some(clip) = clip {
        apply_channels(clip, time, &mut poses, &mut morph_weights)?;
    }
    let mut local_transforms = reserved(poses.len())?;
    for pose in poses {
        let matrix = pose.matrix();
        finite(matrix.into_iter().flatten(), "local transform composition")?;
        local_transforms.push(matrix);
    }
    let world_transforms = world_transforms(asset, &local_transforms)?;
    let mut draws = Vec::new();
    for (instance_index, instance) in asset.instances.iter().enumerate() {
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
                let inverse = inverse_affine(world, instance_index)?;
                let mut palette = reserved(skin.joints.len())?;
                for (&joint, bind) in skin.joints.iter().zip(&skin.inverse_binds) {
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
            if draw.mesh >= asset.mesh_count {
                return Err(EvaluationError::InvalidAsset("draw mesh is out of range"));
            }
            if draw.mesh != instance.mesh {
                continue;
            }
            let weights = &morph_weights[instance_index];
            validate_deformation(
                draw,
                weights.len(),
                skin.as_ref().map(|(_, palette)| palette.len()),
            )?;
            let mut positions = reserved(draw.vertices.len())?;
            let mut world_positions = reserved(draw.vertices.len())?;
            for (vertex_index, vertex) in draw.vertices.iter().enumerate() {
                let mut position = vertex.position;
                for stream in &draw.morphs {
                    if stream.semantic == MorphDeltaSemantic::Position {
                        for (value, delta) in position.iter_mut().zip(stream.values[vertex_index]) {
                            *value += weights[stream.target] * f64::from(delta);
                        }
                    }
                }
                finite(position, "morph evaluation")?;
                let point = [position[0], position[1], position[2], 1.0];
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
                        let joint = usize::try_from(rows.joints[influence])
                            .map_err(|_| EvaluationError::InvalidAsset("joint index overflow"))?;
                        let transformed = transform(palette[joint], point);
                        for (value, component) in blended.iter_mut().zip(transformed) {
                            *value += f64::from(rows.weights[influence]) * component;
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
            }
            draws.try_reserve(1)?;
            draws.push(EvaluatedDraw {
                instance: instance_index,
                draw: draw_index,
                positions,
                world_positions,
            });
        }
    }
    Ok(EvaluatedAsset {
        local_transforms,
        world_transforms,
        morph_weights,
        draws,
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
) -> Result<()> {
    let mut node_targets = filled(poses.len(), 0_u8)?;
    let mut morph_targets = filled(weights.len(), false)?;
    for (index, channel) in clip.channels.iter().enumerate() {
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
        validate_channel(channel, output.len(), index)?;
        sample_channel(channel, time, output, index)?;
    }
    Ok(())
}

fn validate_channel(channel: &Channel, width: usize, index: usize) -> Result<()> {
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
    if channel.values.len() != count || !channel.values.iter().all(|value| value.is_finite()) {
        return Err(invalid("invalid key value table"));
    }
    if !channel.times.iter().all(|time| time.is_finite())
        || channel.times.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(invalid("key times must be finite and strictly increasing"));
    }
    match channel.interpolation {
        AnimationInterpolation::CubicSpline => {
            for tangents in [&channel.in_tangents, &channel.out_tangents] {
                if !tangents.as_ref().is_some_and(|values| {
                    values.len() == count && values.iter().all(|value| value.is_finite())
                }) {
                    return Err(invalid("cubic derivatives must match the key table"));
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

fn sample_channel(channel: &Channel, time: f64, output: &mut [f64], index: usize) -> Result<()> {
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
                // Tangents are raw component derivatives in declared time units, including
                // quaternion derivatives. Never normalize or sign-adjust cubic inputs.
                *out = h00 * f64::from(channel.values[a + lane])
                    + h10 * interval * f64::from(outgoing[a + lane])
                    + h01 * f64::from(channel.values[b + lane])
                    + h11 * interval * f64::from(incoming[b + lane]);
            }
        }
    }
    finite(output.iter().copied(), "curve evaluation")?;
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
) -> Result<Vec<EvaluationMatrix>> {
    let mut parents = filled(local.len(), None)?;
    for (index, node) in asset.nodes.iter().enumerate() {
        for &child in &node.children {
            let parent = parents
                .get_mut(child)
                .ok_or(EvaluationError::InvalidHierarchy)?;
            if parent.replace(index).is_some() {
                return Err(EvaluationError::InvalidHierarchy);
            }
        }
    }
    let mut world = filled(local.len(), IDENTITY)?;
    let mut seen = filled(local.len(), false)?;
    let mut queue = reserved(local.len())?;
    for &root in &asset.roots {
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
        for &child in &asset.nodes[parent].children {
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

fn validate_deformation(draw: &DrawBuffer, width: usize, palette: Option<usize>) -> Result<()> {
    for stream in &draw.morphs {
        if stream.target >= width
            || stream.values.len() != draw.vertices.len()
            || !stream
                .values
                .iter()
                .flatten()
                .all(|value| value.is_finite())
        {
            return Err(EvaluationError::InvalidAsset("invalid morph stream"));
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
            || rows.offsets.windows(2).any(|pair| pair[0] > pair[1])
            || !rows.weights.iter().all(|value| value.is_finite())
            || rows
                .joints
                .iter()
                .any(|joint| usize::try_from(*joint).map_or(true, |joint| joint >= palette))
        {
            return Err(EvaluationError::InvalidAsset("invalid skin influence rows"));
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

fn reserved<T>(capacity: usize) -> Result<Vec<T>> {
    let mut values = Vec::new();
    values.try_reserve_exact(capacity)?;
    Ok(values)
}

fn filled<T: Copy>(len: usize, value: T) -> Result<Vec<T>> {
    let mut values = reserved(len)?;
    values.resize(len, value);
    Ok(values)
}
