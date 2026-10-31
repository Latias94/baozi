use crate::appearance::{
    AlphaMode, AppearanceBuildError, ColorInterpretation, MaterialFactors, MaterialSemantics,
    ShadingModel, TextureBindingSemantics, TextureBlendOperation, TextureChannelSwizzle,
    TextureRole, TextureSourceKey, TextureStrength, TextureTransform, validate_material_semantics,
    validate_texture_binding_semantics,
};
use crate::extension::ExtensionMediaType;
use crate::image::SamplerState;

use super::Presence;
use super::geometry::AuthoringRange;
use super::lineage::ResourceIdentityLocalId;

macro_rules! local_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u32);

        impl $name {
            pub const fn new(index: u32) -> Self {
                Self(index)
            }

            pub const fn index(self) -> u32 {
                self.0
            }
        }
    };
}

local_id!(ImageLocalId);
local_id!(SamplerLocalId);
local_id!(TextureLocalId);
local_id!(MaterialLocalId);

/// Borrowed source payload for one canonical image identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum ImagePayloadAuthoring<'a> {
    Encoded {
        bytes: &'a [u8],
        media_type: Presence<&'a ExtensionMediaType>,
    },
    SessionResource {
        resource: ResourceIdentityLocalId,
        media_type: Presence<&'a ExtensionMediaType>,
    },
}

/// One source image before its payload enters retained storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageAuthoring<'a> {
    payload: ImagePayloadAuthoring<'a>,
}

impl<'a> ImageAuthoring<'a> {
    pub const fn encoded(bytes: &'a [u8], media_type: Presence<&'a ExtensionMediaType>) -> Self {
        Self {
            payload: ImagePayloadAuthoring::Encoded { bytes, media_type },
        }
    }

    /// References bytes snapshotted by the owning import session for one profile resource.
    ///
    /// `resource` must index the resource table in the same `ProfileAuthoringBatch`. The
    /// format producer supplies no stable-identity authority and need not load the resource bytes.
    pub const fn session_resource(
        resource: ResourceIdentityLocalId,
        media_type: Presence<&'a ExtensionMediaType>,
    ) -> Self {
        Self {
            payload: ImagePayloadAuthoring::SessionResource {
                resource,
                media_type,
            },
        }
    }

    pub(in crate::canonical_scene::store) const fn payload(self) -> ImagePayloadAuthoring<'a> {
        self.payload
    }
}

/// One canonical sampler state before retained construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SamplerAuthoring {
    state: SamplerState,
}

impl SamplerAuthoring {
    pub const fn new(state: SamplerState) -> Self {
        Self { state }
    }

    pub(in crate::canonical_scene::store) const fn state(self) -> SamplerState {
        self.state
    }
}

/// One scene-local image/sampler pairing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureAuthoring {
    image: ImageLocalId,
    sampler: SamplerLocalId,
}

impl TextureAuthoring {
    pub const fn new(image: ImageLocalId, sampler: SamplerLocalId) -> Self {
        Self { image, sampler }
    }

    pub(in crate::canonical_scene::store) const fn image(self) -> ImageLocalId {
        self.image
    }

    pub(in crate::canonical_scene::store) const fn sampler(self) -> SamplerLocalId {
        self.sampler
    }
}

/// One material-owned typed texture binding.
///
/// Closed behavior values are owned. Only the optional descriptive source key is borrowed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialBindingAuthoring<'a> {
    texture: TextureLocalId,
    semantics: TextureBindingSemantics<'a>,
}

impl<'a> MaterialBindingAuthoring<'a> {
    /// Creates one binding from public semantic values after checking their cross-field contract.
    ///
    /// `uv_set` is a zero-based UV set independent of any primitive's UV stream count.
    #[allow(clippy::too_many_arguments)]
    pub fn try_from_parts(
        texture: TextureLocalId,
        role: TextureRole,
        uv_set: u16,
        channel_mapping: TextureChannelSwizzle,
        color_interpretation: ColorInterpretation,
        transform: TextureTransform,
        strength: TextureStrength,
        blend: TextureBlendOperation,
        source_key: Option<&'a TextureSourceKey>,
    ) -> Result<Self, AppearanceBuildError> {
        let semantics = TextureBindingSemantics::new(
            role,
            uv_set,
            channel_mapping,
            color_interpretation,
            transform,
            strength,
            blend,
            source_key,
        );
        validate_texture_binding_semantics(semantics)?;
        Ok(Self::new(texture, semantics))
    }

    pub(in crate::canonical_scene::store) const fn new(
        texture: TextureLocalId,
        semantics: TextureBindingSemantics<'a>,
    ) -> Self {
        Self { texture, semantics }
    }

    pub(in crate::canonical_scene::store) const fn texture(self) -> TextureLocalId {
        self.texture
    }

    pub(in crate::canonical_scene::store) const fn semantics(self) -> TextureBindingSemantics<'a> {
        self.semantics
    }
}

/// One logical material owning a contiguous source-ordered binding range.
///
/// Material semantics are owned. The lifetime applies only to an optional source name.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialAuthoring<'a> {
    bindings: AuthoringRange,
    semantics: MaterialSemantics,
    name: Presence<&'a str>,
}

impl<'a> MaterialAuthoring<'a> {
    /// Creates one material from public semantic values after checking required factor groups.
    pub fn try_from_parts(
        bindings: AuthoringRange,
        shading_model: ShadingModel,
        alpha_mode: AlphaMode,
        double_sided: bool,
        factors: MaterialFactors,
    ) -> Result<Self, AppearanceBuildError> {
        let semantics = MaterialSemantics::new(shading_model, alpha_mode, double_sided, factors);
        validate_material_semantics(semantics)?;
        Ok(Self::new(bindings, semantics))
    }

    pub(in crate::canonical_scene::store) const fn new(
        bindings: AuthoringRange,
        semantics: MaterialSemantics,
    ) -> Self {
        Self {
            bindings,
            semantics,
            name: Presence::Absent,
        }
    }

    pub const fn with_name(mut self, name: &'a str) -> Self {
        self.name = Presence::Present(name);
        self
    }

    pub(in crate::canonical_scene::store) const fn bindings(self) -> AuthoringRange {
        self.bindings
    }

    pub(in crate::canonical_scene::store) const fn semantics(self) -> MaterialSemantics {
        self.semantics
    }

    pub(in crate::canonical_scene::store) const fn name(self) -> Presence<&'a str> {
        self.name
    }
}

/// Borrowed appearance-domain input supplied by a format adapter.
#[derive(Debug, Clone, Copy)]
pub struct AppearanceAuthoringBatch<'a> {
    images: &'a [ImageAuthoring<'a>],
    samplers: &'a [SamplerAuthoring],
    textures: &'a [TextureAuthoring],
    bindings: &'a [MaterialBindingAuthoring<'a>],
    materials: &'a [MaterialAuthoring<'a>],
}

impl<'a> AppearanceAuthoringBatch<'a> {
    pub const fn empty() -> Self {
        Self::new(&[], &[], &[], &[], &[])
    }

    pub const fn new(
        images: &'a [ImageAuthoring<'a>],
        samplers: &'a [SamplerAuthoring],
        textures: &'a [TextureAuthoring],
        bindings: &'a [MaterialBindingAuthoring<'a>],
        materials: &'a [MaterialAuthoring<'a>],
    ) -> Self {
        Self {
            images,
            samplers,
            textures,
            bindings,
            materials,
        }
    }

    pub(in crate::canonical_scene::store) const fn images(self) -> &'a [ImageAuthoring<'a>] {
        self.images
    }

    pub(in crate::canonical_scene::store) const fn samplers(self) -> &'a [SamplerAuthoring] {
        self.samplers
    }

    pub(in crate::canonical_scene::store) const fn textures(self) -> &'a [TextureAuthoring] {
        self.textures
    }

    pub(in crate::canonical_scene::store) const fn bindings(
        self,
    ) -> &'a [MaterialBindingAuthoring<'a>] {
        self.bindings
    }

    pub(in crate::canonical_scene::store) const fn materials(self) -> &'a [MaterialAuthoring<'a>] {
        self.materials
    }
}
