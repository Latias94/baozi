/// Maximum encoded bytes owned by one canonical image.
pub const MAX_ENCODED_IMAGE_BYTES: usize = 256 * 1024 * 1024;

/// Magnification or minification filtering within one mip level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TexelFilter {
    Nearest,
    Linear,
}

/// Filtering between mip levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MipmapFilter {
    None,
    Nearest,
    Linear,
}

/// Addressing behavior for one texture coordinate axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TextureAddressMode {
    Repeat,
    MirroredRepeat,
    ClampToEdge,
    ClampToBorder,
}

/// Provenance-free sampler behavior used inside a canonical sampler owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SamplerState {
    minification: TexelFilter,
    magnification: TexelFilter,
    mipmap: MipmapFilter,
    address_u: TextureAddressMode,
    address_v: TextureAddressMode,
    address_w: TextureAddressMode,
}

impl SamplerState {
    /// The canonical behavior used when a source omits sampler state.
    pub const CANONICAL_DEFAULT: Self = Self {
        minification: TexelFilter::Linear,
        magnification: TexelFilter::Linear,
        mipmap: MipmapFilter::Linear,
        address_u: TextureAddressMode::Repeat,
        address_v: TextureAddressMode::Repeat,
        address_w: TextureAddressMode::Repeat,
    };

    /// Creates explicit independent filter and address state.
    pub const fn new(
        minification: TexelFilter,
        magnification: TexelFilter,
        mipmap: MipmapFilter,
        address_u: TextureAddressMode,
        address_v: TextureAddressMode,
        address_w: TextureAddressMode,
    ) -> Self {
        Self {
            minification,
            magnification,
            mipmap,
            address_u,
            address_v,
            address_w,
        }
    }

    pub const fn minification(self) -> TexelFilter {
        self.minification
    }

    pub const fn magnification(self) -> TexelFilter {
        self.magnification
    }

    pub const fn mipmap(self) -> MipmapFilter {
        self.mipmap
    }

    pub const fn address_u(self) -> TextureAddressMode {
        self.address_u
    }

    pub const fn address_v(self) -> TextureAddressMode {
        self.address_v
    }

    pub const fn address_w(self) -> TextureAddressMode {
        self.address_w
    }
}

#[cfg(test)]
mod tests {
    use super::{MipmapFilter, SamplerState, TexelFilter, TextureAddressMode};

    #[test]
    fn canonical_sampler_default_is_explicit() {
        let state = SamplerState::CANONICAL_DEFAULT;

        assert_eq!(state.minification(), TexelFilter::Linear);
        assert_eq!(state.magnification(), TexelFilter::Linear);
        assert_eq!(state.mipmap(), MipmapFilter::Linear);
        assert_eq!(state.address_u(), TextureAddressMode::Repeat);
        assert_eq!(state.address_v(), TextureAddressMode::Repeat);
        assert_eq!(state.address_w(), TextureAddressMode::Repeat);
    }
}
