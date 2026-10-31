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
    /// Return the original Scene unchanged when a NORMAL field exists.
    #[default]
    Preserve,
    /// Explicitly authorize replacing the authored NORMAL values.
    Replace,
}

/// Treatment of a TANGENT field invalidated by a newly generated NORMAL field.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TangentHandling {
    /// Reject the operation when a TANGENT field would become invalid.
    #[default]
    Reject,
    /// Explicitly authorize removing the TANGENT field and its owner-local metadata.
    Remove,
}

/// Concrete normal-generation policy. Replacing source fields requires an explicit choice.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NormalOptions {
    /// Flat face normals or area-weighted smoothing by shared vertex identity.
    pub mode: NormalMode,
    /// Preserve or explicitly replace an authored NORMAL field.
    pub existing: ExistingNormals,
    /// Reject or explicitly remove an invalidated TANGENT field.
    pub tangents: TangentHandling,
}

impl NormalOptions {
    pub(crate) fn digest(self) -> baozi_core::__scene_integration::authoring::OptionsDigest {
        // SHA-256 of the exact durable instruction strings listed in this table's order.
        const DIGESTS: [[u8; 32]; 8] = [
            // baozi.generate_normals/v1;mode=flat;existing=preserve;tangents=reject;smooth=area-weighted-vertex-group
            [
                0x4a, 0x05, 0x21, 0x69, 0x4d, 0xf9, 0x46, 0xbf, 0xff, 0x50, 0xc3, 0xf5, 0x86, 0x65,
                0xf9, 0xd8, 0x78, 0x51, 0x2c, 0x00, 0x1b, 0x1f, 0x90, 0xa0, 0x3a, 0x34, 0x33, 0x17,
                0x3b, 0x8e, 0x80, 0x8c,
            ],
            // baozi.generate_normals/v1;mode=flat;existing=preserve;tangents=remove;smooth=area-weighted-vertex-group
            [
                0x1b, 0xa0, 0x6a, 0x3a, 0x30, 0x65, 0x28, 0x1c, 0x25, 0x54, 0xc4, 0x2e, 0xa1, 0xbd,
                0x84, 0xbf, 0x8a, 0xf3, 0xbe, 0x6f, 0xd3, 0x2e, 0x9c, 0x56, 0xa4, 0x30, 0x0e, 0x1f,
                0xb0, 0xb3, 0x86, 0xb7,
            ],
            // baozi.generate_normals/v1;mode=flat;existing=replace;tangents=reject;smooth=area-weighted-vertex-group
            [
                0x39, 0x24, 0x5b, 0x33, 0x2c, 0x89, 0x64, 0x0c, 0x04, 0x78, 0x35, 0x4f, 0x09, 0x73,
                0x63, 0x09, 0xb3, 0xac, 0xe6, 0xd6, 0xbd, 0x04, 0xd6, 0xca, 0x8d, 0x41, 0x3d, 0xa5,
                0xc4, 0x75, 0x1a, 0xd4,
            ],
            // baozi.generate_normals/v1;mode=flat;existing=replace;tangents=remove;smooth=area-weighted-vertex-group
            [
                0x34, 0x7a, 0x06, 0x47, 0xfa, 0xf0, 0xb0, 0x60, 0x87, 0x91, 0x91, 0xda, 0xc0, 0x26,
                0xd3, 0x30, 0xbb, 0x77, 0xd0, 0x9f, 0xa4, 0x00, 0x58, 0x86, 0xd4, 0xa3, 0x04, 0x2f,
                0x70, 0x0d, 0x9d, 0xcf,
            ],
            // baozi.generate_normals/v1;mode=smooth;existing=preserve;tangents=reject;smooth=area-weighted-vertex-group
            [
                0x5d, 0x2d, 0x5e, 0x86, 0x5f, 0x29, 0x5d, 0x91, 0x5e, 0xbb, 0xca, 0x88, 0xa2, 0x0e,
                0x0d, 0x9d, 0x9e, 0xfb, 0x65, 0xba, 0xc3, 0xad, 0xba, 0x38, 0x49, 0x8e, 0x0c, 0x64,
                0x7a, 0xe9, 0x42, 0xcb,
            ],
            // baozi.generate_normals/v1;mode=smooth;existing=preserve;tangents=remove;smooth=area-weighted-vertex-group
            [
                0x1f, 0x8d, 0x0b, 0x15, 0x33, 0xb0, 0x0c, 0x44, 0xe2, 0x0d, 0x03, 0xa6, 0xe8, 0xf2,
                0x11, 0x3c, 0x01, 0x62, 0x81, 0x28, 0xd8, 0x68, 0xe5, 0x2d, 0x98, 0x0e, 0xbf, 0x4e,
                0x78, 0x77, 0x71, 0x45,
            ],
            // baozi.generate_normals/v1;mode=smooth;existing=replace;tangents=reject;smooth=area-weighted-vertex-group
            [
                0x35, 0x2b, 0x0f, 0xdd, 0xcc, 0x7f, 0x7a, 0xdc, 0x4a, 0x16, 0x5c, 0xa0, 0xaa, 0x6f,
                0xf0, 0x8b, 0xff, 0xdc, 0x9e, 0x3d, 0xaa, 0x3c, 0x4f, 0x79, 0x7f, 0xdf, 0x10, 0x15,
                0x81, 0x21, 0xf8, 0x78,
            ],
            // baozi.generate_normals/v1;mode=smooth;existing=replace;tangents=remove;smooth=area-weighted-vertex-group
            [
                0x3a, 0xfe, 0xab, 0xde, 0x48, 0x60, 0x44, 0xc0, 0x54, 0x30, 0x0a, 0x1d, 0xf4, 0xfe,
                0x0f, 0xde, 0x2d, 0x6c, 0x7b, 0x42, 0x84, 0x16, 0x50, 0x9d, 0xeb, 0x4c, 0x04, 0x59,
                0x4d, 0xd1, 0x89, 0xa7,
            ],
        ];
        let index = usize::from(self.mode == NormalMode::Smooth) * 4
            + usize::from(self.existing == ExistingNormals::Replace) * 2
            + usize::from(self.tangents == TangentHandling::Remove);
        baozi_core::__scene_integration::authoring::OptionsDigest::sha256(DIGESTS[index])
    }
}
