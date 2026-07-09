#![forbid(unsafe_code)]

use baozi_core::{BaoziError, Result, Scene};
use baozi_import::{
    CapabilityStatus, FormatCapability, FormatEncoding, FormatImporter, FormatInfo, FormatMaturity,
    FormatSidecarPolicy, ImportContext, ImporterRegistry,
};

pub struct GltfImporter;

pub fn format_info() -> FormatInfo {
    FormatInfo::new("gltf", "glTF 2.0", &["gltf", "glb"])
        .with_media_types(&["model/gltf+json", "model/gltf-binary"])
        .with_encoding(FormatEncoding::TextOrBinary)
        .with_sidecar_policy(FormatSidecarPolicy::ExternalBuffers)
        .with_maturity(FormatMaturity::Experimental)
        .with_capabilities(&[
            (FormatCapability::Geometry, CapabilityStatus::Unknown),
            (FormatCapability::Hierarchy, CapabilityStatus::Unknown),
            (FormatCapability::Materials, CapabilityStatus::Unknown),
            (FormatCapability::Textures, CapabilityStatus::Unknown),
            (FormatCapability::CamerasLights, CapabilityStatus::Unknown),
            (FormatCapability::Animation, CapabilityStatus::Unknown),
            (FormatCapability::Skinning, CapabilityStatus::Unknown),
            (FormatCapability::MorphTargets, CapabilityStatus::Unknown),
            (FormatCapability::Metadata, CapabilityStatus::Unknown),
            (
                FormatCapability::CompressionContainers,
                CapabilityStatus::Unknown,
            ),
            (
                FormatCapability::CoordinatesUnits,
                CapabilityStatus::Unknown,
            ),
            (FormatCapability::Diagnostics, CapabilityStatus::Unknown),
            (FormatCapability::ResourceLimits, CapabilityStatus::Unknown),
        ])
        .with_notes("planned glTF importer shell; parsing is not implemented yet")
        .with_docs_path("docs/formats/gltf.md")
}

pub fn register(registry: &mut ImporterRegistry) -> Result<()> {
    registry.register(GltfImporter)
}

impl FormatImporter for GltfImporter {
    fn info(&self) -> FormatInfo {
        format_info()
    }

    fn read(&self, _ctx: &mut ImportContext<'_>) -> Result<Scene> {
        Err(BaoziError::unsupported_format(
            "gltf parser not implemented",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baozi_test_support::{SupportMatrixColumn, assert_support_matrix_row};

    #[test]
    fn support_matrix_matches_format_info() {
        assert_support_matrix_row(
            "baozi-format-gltf",
            &format_info(),
            &[
                (SupportMatrixColumn::Geometry, FormatCapability::Geometry),
                (SupportMatrixColumn::Materials, FormatCapability::Materials),
                (SupportMatrixColumn::Textures, FormatCapability::Textures),
                (SupportMatrixColumn::Animation, FormatCapability::Animation),
                (
                    SupportMatrixColumn::Diagnostics,
                    FormatCapability::Diagnostics,
                ),
            ],
        );
    }
}
