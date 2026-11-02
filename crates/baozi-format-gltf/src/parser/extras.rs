//! Descriptive source extras, preserved as exact JSON values without a second JSON model.

use baozi_import::ParseExecution;
use baozi_import::authoring::NamespacedName;

use super::{Result, checked_add_with_limit, enforce_usize_limit, gltf_parse_error};
use crate::source_map::{SourceEntityId, SourcePayload, SourcePayloadSite, SourceRange};

pub(super) const MEDIA_TYPE: &str = "application/json";

pub(super) struct SourceExtras {
    pub(super) source: SourceEntityId,
    pub(super) key: NamespacedName,
    pub(super) range: SourceRange,
}

impl SourceExtras {
    pub(super) fn new(_ctx: &mut ParseExecution<'_, '_>, payload: &SourcePayload) -> Result<Self> {
        Ok(Self {
            source: payload.owner,
            key: NamespacedName::new(key(payload.site))
                .map_err(|_| gltf_parse_error("static glTF extras key is invalid"))?,
            range: payload.value_range,
        })
    }

    pub(super) fn bytes<'a>(&self, ctx: &mut ParseExecution<'_, 'a>) -> Result<&'a [u8]> {
        let start = usize::try_from(self.range.byte_offset).ok();
        let length = usize::try_from(self.range.byte_length).ok();
        start
            .zip(length)
            .and_then(|(start, length)| start.checked_add(length).map(|end| start..end))
            .and_then(|range| ctx.primary_bytes().get(range))
            .ok_or_else(|| gltf_parse_error("glTF extras range exceeds the primary snapshot"))
    }
}

/// A lower-bound preflight before dependency IO. Core independently admits the complete metadata
/// batch, including other source properties, and meters every retained key/media/payload byte.
#[derive(Default)]
pub(super) struct ExtrasPreflight {
    entries: usize,
    bytes: usize,
}

impl ExtrasPreflight {
    pub(super) fn add(
        &mut self,
        ctx: &mut ParseExecution<'_, '_>,
        payload: &SourcePayload,
    ) -> Result<()> {
        let name = key(payload.site);
        {
            let argument_0 = ctx.limits().max_string_bytes;
            enforce_usize_limit(ctx, "max_string_bytes", name.len(), argument_0)
        }?;
        self.entries = {
            let argument_0 = ctx.limits().max_records;
            checked_add_with_limit(ctx, "max_records", self.entries, 1, argument_0)
        }?;
        let payload_bytes = usize::try_from(payload.value_range.byte_length)
            .map_err(|_| ctx.allocation_failure("gltf-extras-length"))?;
        for bytes in [name.len(), MEDIA_TYPE.len(), payload_bytes] {
            self.bytes = {
                let argument_0 = ctx.limits().max_canonical_string_bytes;
                checked_add_with_limit(
                    ctx,
                    "max_canonical_string_bytes",
                    self.bytes,
                    bytes,
                    argument_0,
                )
            }?;
        }
        Ok(())
    }
}

/// Keys name the source object's schema site, never a canonical Scene index. The metadata owner
/// supplies the source identity, including the parent ordinal of a primitive/channel/sampler.
fn key(site: SourcePayloadSite) -> &'static str {
    use SourcePayloadSite as Site;
    match site {
        Site::Entity => "org.baozi.gltf:extras",
        Site::Asset => "org.baozi.gltf:asset-extras",
        Site::MaterialPbrMetallicRoughness => "org.baozi.gltf:pbr-metallic-roughness-extras",
        Site::MaterialUnlit => "org.baozi.gltf:unlit-extras",
        Site::MaterialEmissiveStrength => "org.baozi.gltf:emissive-strength-extras",
        Site::MaterialClearcoat => "org.baozi.gltf:clearcoat-extras",
        Site::MaterialSheen => "org.baozi.gltf:sheen-extras",
        Site::BaseColorTexture => "org.baozi.gltf:base-color-texture-extras",
        Site::MetallicRoughnessTexture => "org.baozi.gltf:metallic-roughness-texture-extras",
        Site::NormalTexture => "org.baozi.gltf:normal-texture-extras",
        Site::OcclusionTexture => "org.baozi.gltf:occlusion-texture-extras",
        Site::EmissiveTexture => "org.baozi.gltf:emissive-texture-extras",
        Site::ClearcoatTexture => "org.baozi.gltf:clearcoat-texture-extras",
        Site::ClearcoatRoughnessTexture => "org.baozi.gltf:clearcoat-roughness-texture-extras",
        Site::ClearcoatNormalTexture => "org.baozi.gltf:clearcoat-normal-texture-extras",
        Site::SheenColorTexture => "org.baozi.gltf:sheen-color-texture-extras",
        Site::SheenRoughnessTexture => "org.baozi.gltf:sheen-roughness-texture-extras",
        Site::BaseColorTextureTransform => "org.baozi.gltf:base-color-texture-transform-extras",
        Site::MetallicRoughnessTextureTransform => {
            "org.baozi.gltf:metallic-roughness-texture-transform-extras"
        }
        Site::NormalTextureTransform => "org.baozi.gltf:normal-texture-transform-extras",
        Site::OcclusionTextureTransform => "org.baozi.gltf:occlusion-texture-transform-extras",
        Site::EmissiveTextureTransform => "org.baozi.gltf:emissive-texture-transform-extras",
        Site::ClearcoatTextureTransform => "org.baozi.gltf:clearcoat-texture-transform-extras",
        Site::ClearcoatRoughnessTextureTransform => {
            "org.baozi.gltf:clearcoat-roughness-texture-transform-extras"
        }
        Site::ClearcoatNormalTextureTransform => {
            "org.baozi.gltf:clearcoat-normal-texture-transform-extras"
        }
        Site::SheenColorTextureTransform => "org.baozi.gltf:sheen-color-texture-transform-extras",
        Site::SheenRoughnessTextureTransform => {
            "org.baozi.gltf:sheen-roughness-texture-transform-extras"
        }
        Site::AccessorSparse => "org.baozi.gltf:sparse-extras",
        Site::AccessorSparseIndices => "org.baozi.gltf:sparse-indices-extras",
        Site::AccessorSparseValues => "org.baozi.gltf:sparse-values-extras",
        Site::AnimationChannelTarget => "org.baozi.gltf:target-extras",
        Site::CameraOrthographic => "org.baozi.gltf:orthographic-extras",
        Site::CameraPerspective => "org.baozi.gltf:perspective-extras",
        Site::DocumentPunctualLights => "org.baozi.gltf:punctual-lights-extras",
        Site::NodePunctualLight => "org.baozi.gltf:punctual-light-extras",
        Site::LightSpot => "org.baozi.gltf:spot-extras",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clearcoat_sheen_extras_keys_identify_each_nested_site() {
        use SourcePayloadSite as Site;
        let expected = [
            (Site::MaterialClearcoat, "org.baozi.gltf:clearcoat-extras"),
            (Site::MaterialSheen, "org.baozi.gltf:sheen-extras"),
            (
                Site::ClearcoatTexture,
                "org.baozi.gltf:clearcoat-texture-extras",
            ),
            (
                Site::ClearcoatRoughnessTexture,
                "org.baozi.gltf:clearcoat-roughness-texture-extras",
            ),
            (
                Site::ClearcoatNormalTexture,
                "org.baozi.gltf:clearcoat-normal-texture-extras",
            ),
            (
                Site::SheenColorTexture,
                "org.baozi.gltf:sheen-color-texture-extras",
            ),
            (
                Site::SheenRoughnessTexture,
                "org.baozi.gltf:sheen-roughness-texture-extras",
            ),
            (
                Site::ClearcoatTextureTransform,
                "org.baozi.gltf:clearcoat-texture-transform-extras",
            ),
            (
                Site::ClearcoatRoughnessTextureTransform,
                "org.baozi.gltf:clearcoat-roughness-texture-transform-extras",
            ),
            (
                Site::ClearcoatNormalTextureTransform,
                "org.baozi.gltf:clearcoat-normal-texture-transform-extras",
            ),
            (
                Site::SheenColorTextureTransform,
                "org.baozi.gltf:sheen-color-texture-transform-extras",
            ),
            (
                Site::SheenRoughnessTextureTransform,
                "org.baozi.gltf:sheen-roughness-texture-transform-extras",
            ),
        ];
        for (site, expected) in expected {
            assert_eq!(key(site), expected);
            assert!(NamespacedName::new(expected).is_ok());
        }
    }
}
