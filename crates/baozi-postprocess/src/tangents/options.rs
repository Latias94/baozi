use baozi_core::AttributeDomain;
use sha2::{Digest, Sha256};

/// Treatment of authored tangent streams on the selected primitive.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExistingTangents {
    /// Return the original Scene unchanged if any Tangent stream already exists.
    #[default]
    Preserve,
    /// Generate when absent or replace one authored Tangent stream. Multiple streams reject.
    Replace,
}

/// Explicit field selection for the Triangle UV Derivative v1 algorithm.
///
/// The selected Normal and Texcoord domains are independent. Vertex and Corner reads
/// preserve their existing indexing; this operation never guesses a preferred domain or UV set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TangentOptions {
    pub normal_domain: AttributeDomain,
    pub uv_set: u32,
    pub uv_domain: AttributeDomain,
    pub existing: ExistingTangents,
}

impl TangentOptions {
    pub const fn new(
        normal_domain: AttributeDomain,
        uv_set: u32,
        uv_domain: AttributeDomain,
    ) -> Self {
        Self {
            normal_domain,
            uv_set,
            uv_domain,
            existing: ExistingTangents::Preserve,
        }
    }

    pub(crate) fn digest(self) -> baozi_core::__scene_integration::authoring::OptionsDigest {
        let mut hash = Sha256::new();
        hash.update(b"baozi.generate_tangents/triangle-uv-derivative-v1\0");
        hash.update([domain(self.normal_domain), domain(self.uv_domain)]);
        hash.update(self.uv_set.to_le_bytes());
        hash.update([u8::from(self.existing == ExistingTangents::Replace)]);
        baozi_core::__scene_integration::authoring::OptionsDigest::sha256(hash.finalize().into())
    }
}

const fn domain(value: AttributeDomain) -> u8 {
    match value {
        AttributeDomain::Vertex => 0,
        AttributeDomain::Corner => 1,
        AttributeDomain::Face => 2,
        AttributeDomain::Instance => 3,
    }
}
