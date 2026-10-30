//! Constant-time row/range selection. No preceding row, scalar stream, or proof is decoded.

use super::*;

impl<'scene> NodeSceneAnimationTables<'scene> {
    pub(in crate::canonical_scene::store) fn clip_at(
        self,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneAnimationClip<'scene>> {
        let clip = self
            .storage
            .clips
            .get(row)
            .ok_or_else(|| invariant("animation clip is outside its proof-qualified table"))?;
        record_row_read();
        if !clip.duration.is_finite() || clip.duration < 0.0 || !valid_time_domain(clip.time_domain)
        {
            return Err(invariant(
                "validated animation clip duration or rate is invalid",
            ));
        }
        let channels = borrow_range(clip.channels, &self.storage.channels)?;
        let name = clip
            .name
            .map(|range| {
                range
                    .get(&self.storage.names)
                    .ok_or_else(|| invariant("validated animation clip name is unavailable"))
            })
            .transpose()?;
        Ok(NodeSceneAnimationClip {
            name,
            source_index: clip.source_index as usize,
            duration: clip.duration,
            time_domain: clip.time_domain,
            channel_offset: clip.channels.start(),
            channel_count: channels.len(),
        })
    }

    pub(in crate::canonical_scene::store) fn channel_at(
        self,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneAnimationChannel<'scene>> {
        let channel =
            self.storage.channels.get(row).ok_or_else(|| {
                invariant("animation channel is outside its proof-qualified table")
            })?;
        record_row_read();
        let target = self.checked_target(channel.target)?;
        let times = borrow_range(channel.times, &self.storage.times)?;
        let values = borrow_range(channel.values, &self.storage.values)?;
        let value_width = channel
            .value_width()
            .ok_or_else(|| invariant("validated animation channel sample shape is inconsistent"))?
            as usize;
        let in_tangents = borrow_optional_range(channel.in_tangents, &self.storage.in_tangents)?;
        let out_tangents = borrow_optional_range(channel.out_tangents, &self.storage.out_tangents)?;
        match (channel.interpolation, in_tangents, out_tangents) {
            (AnimationInterpolation::CubicSpline, Some(input), Some(output))
                if input.len() == values.len() && output.len() == values.len() => {}
            (AnimationInterpolation::Step | AnimationInterpolation::Linear, None, None) => {}
            _ => {
                return Err(invariant(
                    "validated animation tangent presence or shape is inconsistent",
                ));
            }
        }
        Ok(NodeSceneAnimationChannel {
            source_index: channel.source_index as usize,
            target,
            interpolation: channel.interpolation,
            pre_extrapolation: channel.pre_extrapolation,
            post_extrapolation: channel.post_extrapolation,
            value_width,
            times,
            values,
            in_tangents,
            out_tangents,
        })
    }

    fn checked_target(
        self,
        target: StoredAnimationTarget,
    ) -> NodeSceneSemanticResult<NodeSceneAnimationTarget> {
        match target {
            StoredAnimationTarget::NodeTranslation(node) => Ok(
                NodeSceneAnimationTarget::NodeTranslation(self.canonical_node(node)?),
            ),
            StoredAnimationTarget::NodeRotation(node) => Ok(
                NodeSceneAnimationTarget::NodeRotation(self.canonical_node(node)?),
            ),
            StoredAnimationTarget::NodeScale(node) => Ok(NodeSceneAnimationTarget::NodeScale(
                self.canonical_node(node)?,
            )),
            StoredAnimationTarget::MeshInstanceMorphWeights(instance) => {
                let index = instance.0 as usize;
                if index >= self.mesh_instance_count {
                    return Err(invariant(
                        "validated animation morph target is outside mesh-instance space",
                    ));
                }
                Ok(NodeSceneAnimationTarget::MeshInstanceMorphWeights(index))
            }
        }
    }

    fn canonical_node(self, node: StoredNodeIndex) -> NodeSceneSemanticResult<usize> {
        let index = node.0 as usize;
        if index == 0 || index > self.source_node_count {
            return Err(invariant(
                "validated animation target is outside canonical node space",
            ));
        }
        Ok(index)
    }
}

fn valid_time_domain(domain: AnimationTimeDomain) -> bool {
    match domain {
        AnimationTimeDomain::Seconds | AnimationTimeDomain::Ticks { rate: None } => true,
        AnimationTimeDomain::Frames { rate } | AnimationTimeDomain::Ticks { rate: Some(rate) } => {
            rate.is_finite() && rate > 0.0
        }
    }
}
