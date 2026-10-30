//! Appearance rows and compact byte/text pools in one retained Scene owner.
//!
//! Indices are generation-independent logical rows, never record-store references.
//! Shared public behavior values are stored directly after their domain shrink.

use crate::appearance::{
    AlphaMode, ColorInterpretation, MaterialFactors, MaterialSemanticsRef, ShadingModel,
    TextureBindingSemanticsRef, TextureBlendOperation, TextureChannelSwizzle,
    TextureCoordinateSource, TextureRole, TextureSourceKey, TextureStrength, TextureTransform,
};
use crate::image::SamplerState;

use super::{TableRange, TextRange};

pub(in crate::canonical_scene::store) mod build;
pub(in crate::canonical_scene::store) mod validation;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::canonical_scene::store) struct StoredImageIndex(
    pub(in crate::canonical_scene::store) u32,
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::canonical_scene::store) struct StoredSamplerIndex(
    pub(in crate::canonical_scene::store) u32,
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::canonical_scene::store) struct StoredTextureIndex(
    pub(in crate::canonical_scene::store) u32,
);

/// Session-resource and directly encoded inputs both retain their exact resolved
/// encoded bytes. Source-resource identity remains in profile ownership/provenance.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct StoredImage {
    pub(in crate::canonical_scene::store) payload: TableRange<u8>,
    pub(in crate::canonical_scene::store) media_type: Option<TextRange>,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct StoredTexture {
    pub(in crate::canonical_scene::store) image: StoredImageIndex,
    pub(in crate::canonical_scene::store) sampler: StoredSamplerIndex,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct StoredBinding {
    pub(in crate::canonical_scene::store) texture: StoredTextureIndex,
    pub(in crate::canonical_scene::store) role: TextureRole,
    pub(in crate::canonical_scene::store) coordinate_source: TextureCoordinateSource,
    pub(in crate::canonical_scene::store) channels: TextureChannelSwizzle,
    pub(in crate::canonical_scene::store) color: ColorInterpretation,
    pub(in crate::canonical_scene::store) transform: TextureTransform,
    pub(in crate::canonical_scene::store) strength: TextureStrength,
    pub(in crate::canonical_scene::store) blend: TextureBlendOperation,
    pub(in crate::canonical_scene::store) source_key: Option<TextRange>,
}

impl StoredBinding {
    /// Borrows the row's behavior values and an optional descriptive source key.
    /// Central validation checks the actual key text separately without copying
    /// its bounded buffer into every row or granting it binding authority.
    pub(in crate::canonical_scene::store) const fn semantics<'view>(
        &'view self,
        source_key: Option<&'view TextureSourceKey>,
    ) -> TextureBindingSemanticsRef<'view> {
        TextureBindingSemanticsRef::new(
            &self.role,
            &self.coordinate_source,
            &self.channels,
            &self.color,
            self.transform,
            self.strength,
            &self.blend,
            source_key,
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct StoredMaterial {
    pub(in crate::canonical_scene::store) bindings: TableRange<StoredBinding>,
    pub(in crate::canonical_scene::store) shading: ShadingModel,
    pub(in crate::canonical_scene::store) alpha: AlphaMode,
    pub(in crate::canonical_scene::store) double_sided: bool,
    pub(in crate::canonical_scene::store) factors: MaterialFactors,
    pub(in crate::canonical_scene::store) name: Option<TextRange>,
}

impl StoredMaterial {
    pub(in crate::canonical_scene::store) const fn semantics(&self) -> MaterialSemanticsRef<'_> {
        MaterialSemanticsRef::new(&self.shading, &self.alpha, self.double_sided, self.factors)
    }
}

/// Exactly five row tables and two pools, without nested allocations.
///
/// Row order is source order. image_bytes concatenates every resolved image payload
/// in image-row order. text concatenates all image media types, then all binding
/// source keys, then all material names, preserving row order within each section.
/// Text is not interned or deduplicated. Option<TextRange> preserves absence and an
/// explicitly authored empty name; media types and source keys retain their own
/// public construction constraints. Inline factors/behavior values are counted once.
pub(in crate::canonical_scene::store) struct AppearanceStorage {
    pub(in crate::canonical_scene::store) images: Vec<StoredImage>,
    pub(in crate::canonical_scene::store) samplers: Vec<SamplerState>,
    pub(in crate::canonical_scene::store) textures: Vec<StoredTexture>,
    pub(in crate::canonical_scene::store) bindings: Vec<StoredBinding>,
    pub(in crate::canonical_scene::store) materials: Vec<StoredMaterial>,
    pub(in crate::canonical_scene::store) image_bytes: Vec<u8>,
    pub(in crate::canonical_scene::store) text: Vec<u8>,
}
