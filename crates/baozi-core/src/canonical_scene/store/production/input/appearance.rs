use crate::appearance::TextureRole;
use crate::extension::ExtensionMediaType;
use crate::image::SamplerState;

use super::Presence;
use super::geometry::AuthoringRange;

macro_rules! local_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub(in crate::canonical_scene::store) struct $name(u32);

        impl $name {
            pub(in crate::canonical_scene::store) const fn new(index: u32) -> Self {
                Self(index)
            }

            pub(in crate::canonical_scene::store) const fn index(self) -> u32 {
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
}

/// One source image before its payload enters retained storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct ImageAuthoring<'a> {
    payload: ImagePayloadAuthoring<'a>,
}

impl<'a> ImageAuthoring<'a> {
    pub(in crate::canonical_scene::store) const fn encoded(
        bytes: &'a [u8],
        media_type: Presence<&'a ExtensionMediaType>,
    ) -> Self {
        Self {
            payload: ImagePayloadAuthoring::Encoded { bytes, media_type },
        }
    }

    pub(in crate::canonical_scene::store) const fn payload(self) -> ImagePayloadAuthoring<'a> {
        self.payload
    }
}

/// One canonical sampler state before retained construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct SamplerAuthoring {
    state: SamplerState,
}

impl SamplerAuthoring {
    pub(in crate::canonical_scene::store) const fn new(state: SamplerState) -> Self {
        Self { state }
    }

    pub(in crate::canonical_scene::store) const fn state(self) -> SamplerState {
        self.state
    }
}

/// One scene-local image/sampler pairing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct TextureAuthoring {
    image: ImageLocalId,
    sampler: SamplerLocalId,
}

impl TextureAuthoring {
    pub(in crate::canonical_scene::store) const fn new(
        image: ImageLocalId,
        sampler: SamplerLocalId,
    ) -> Self {
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct MaterialBindingAuthoring<'a> {
    texture: TextureLocalId,
    role: &'a TextureRole,
}

impl<'a> MaterialBindingAuthoring<'a> {
    pub(in crate::canonical_scene::store) const fn new(
        texture: TextureLocalId,
        role: &'a TextureRole,
    ) -> Self {
        Self { texture, role }
    }

    pub(in crate::canonical_scene::store) const fn texture(self) -> TextureLocalId {
        self.texture
    }

    pub(in crate::canonical_scene::store) const fn role(self) -> &'a TextureRole {
        self.role
    }
}

/// One logical material owning a contiguous source-ordered binding range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct MaterialAuthoring {
    bindings: AuthoringRange,
}

impl MaterialAuthoring {
    pub(in crate::canonical_scene::store) const fn new(bindings: AuthoringRange) -> Self {
        Self { bindings }
    }

    pub(in crate::canonical_scene::store) const fn bindings(self) -> AuthoringRange {
        self.bindings
    }
}

/// Borrowed appearance-domain input supplied by a format adapter.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct AppearanceAuthoringBatch<'a> {
    images: &'a [ImageAuthoring<'a>],
    samplers: &'a [SamplerAuthoring],
    textures: &'a [TextureAuthoring],
    bindings: &'a [MaterialBindingAuthoring<'a>],
    materials: &'a [MaterialAuthoring],
}

impl<'a> AppearanceAuthoringBatch<'a> {
    pub(in crate::canonical_scene::store) const fn empty() -> Self {
        Self::new(&[], &[], &[], &[], &[])
    }

    pub(in crate::canonical_scene::store) const fn new(
        images: &'a [ImageAuthoring<'a>],
        samplers: &'a [SamplerAuthoring],
        textures: &'a [TextureAuthoring],
        bindings: &'a [MaterialBindingAuthoring<'a>],
        materials: &'a [MaterialAuthoring],
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

    pub(in crate::canonical_scene::store) const fn materials(self) -> &'a [MaterialAuthoring] {
        self.materials
    }
}
