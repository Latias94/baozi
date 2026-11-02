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
        h.update(b"baozi.generate-normals/v1\0");
        h.update([
            u8::from(self.mode == NormalMode::Smooth),
            u8::from(self.existing == ExistingNormals::Replace),
        ]);
        baozi_core::__scene_integration::authoring::OptionsDigest::sha256(h.finalize().into())
    }
}

#[cfg(test)]
mod tests {
    use super::{ExistingNormals, NormalMode, NormalOptions};

    #[test]
    fn digest_matches_independent_v1_mode_and_replacement_vectors() {
        // Independent SHA-256 of the NUL-terminated v1 domain, then mode/replacement bytes.
        for (mode, existing, expected) in [
            (
                NormalMode::Flat,
                ExistingNormals::Preserve,
                [
                    0x4b, 0xc9, 0x34, 0x8a, 0x19, 0x35, 0xbe, 0x6a, 0xd1, 0x40, 0x73, 0xa5, 0x27,
                    0x7f, 0xaf, 0x9d, 0x06, 0x42, 0x93, 0x77, 0x12, 0xe2, 0x14, 0x5c, 0x7f, 0x0f,
                    0x27, 0xd2, 0xc7, 0x19, 0xbb, 0x62,
                ],
            ),
            (
                NormalMode::Flat,
                ExistingNormals::Replace,
                [
                    0xa0, 0x42, 0x7b, 0x8b, 0xe2, 0x2f, 0x62, 0xbb, 0x42, 0x46, 0x27, 0xa0, 0x3e,
                    0x21, 0x03, 0x7a, 0xbc, 0x95, 0x76, 0x29, 0x02, 0xd9, 0x33, 0xd7, 0xd6, 0x70,
                    0x8b, 0x31, 0x05, 0x0a, 0xd0, 0xfe,
                ],
            ),
            (
                NormalMode::Smooth,
                ExistingNormals::Preserve,
                [
                    0xe1, 0x71, 0xb0, 0x48, 0xa0, 0xc9, 0x8d, 0x5c, 0x55, 0xeb, 0x15, 0x90, 0x04,
                    0x7f, 0xc6, 0x07, 0x24, 0xfe, 0xe9, 0x77, 0x0e, 0x3c, 0x22, 0x65, 0x9b, 0xbe,
                    0xef, 0x9b, 0x41, 0x7b, 0x08, 0xd2,
                ],
            ),
            (
                NormalMode::Smooth,
                ExistingNormals::Replace,
                [
                    0xcf, 0x25, 0xd5, 0x24, 0xc0, 0xa0, 0xb9, 0x63, 0xd5, 0xf0, 0xe7, 0xc1, 0x00,
                    0x7c, 0xdd, 0x45, 0xc3, 0x75, 0xb5, 0xe8, 0x8d, 0x1f, 0x39, 0x38, 0x37, 0xe7,
                    0x68, 0x09, 0x1c, 0xfc, 0x9a, 0xd6,
                ],
            ),
        ] {
            assert_eq!(
                NormalOptions { mode, existing }.digest().as_bytes(),
                &expected,
                "{mode:?}/{existing:?}",
            );
        }
    }
}
