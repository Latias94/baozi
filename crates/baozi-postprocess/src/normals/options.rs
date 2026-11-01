/// The normal field generated for a triangle primitive.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NormalMode {
    /// Every triangle has a separate face normal, stored per corner.
    Flat,
    /// Shared vertex identities and nonzero Face smoothing groups share area-weighted normals.
    #[default]
    Smooth,
}

/// Treatment of a primitive that already has an authored NORMAL field.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExistingNormals {
    /// Keep an existing NORMAL field. Other explicitly requested preparation stages may change
    /// the Scene even when this field is preserved.
    #[default]
    Preserve,
    /// Explicitly authorize replacing the authored NORMAL values.
    Replace,
}

/// Concrete normal-generation policy. Replacing source fields requires an explicit choice.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NormalOptions {
    /// Flat face normals or area-weighted smoothing by shared vertex identity.
    pub mode: NormalMode,
    /// Preserve or explicitly replace an authored NORMAL field.
    pub existing: ExistingNormals,
}

impl NormalOptions {
    pub(crate) fn digest(self) -> baozi_core::__scene_integration::authoring::OptionsDigest {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"baozi.generate-normals/v2\0");
        h.update([
            u8::from(self.mode == NormalMode::Smooth),
            u8::from(self.existing == ExistingNormals::Replace),
        ]);
        baozi_core::__scene_integration::authoring::OptionsDigest::sha256(h.finalize().into())
    }
}
