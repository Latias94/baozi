#![forbid(unsafe_code)]

mod detect;
mod parser;

use baozi_core::{Result, Scene};
use baozi_import::{
    CapabilityStatus, FormatCapability, FormatEncoding, FormatImporter, FormatInfo, FormatMaturity,
    FormatSidecarPolicy, ImportContext, ImporterRegistry, ReadConfidence, ReadHint,
};
use baozi_io::ReadSeek;

pub struct GltfImporter;

pub fn format_info() -> FormatInfo {
    FormatInfo::new("gltf", "glTF 2.0", &["gltf", "glb"])
        .with_media_types(&["model/gltf+json", "model/gltf-binary"])
        .with_encoding(FormatEncoding::TextOrBinary)
        .with_sidecar_policy(FormatSidecarPolicy::ExternalBuffers)
        .with_maturity(FormatMaturity::Experimental)
        .with_capabilities(&[
            (FormatCapability::Geometry, CapabilityStatus::Supported),
            (FormatCapability::Hierarchy, CapabilityStatus::Partial),
            (FormatCapability::Materials, CapabilityStatus::Partial),
            (FormatCapability::Textures, CapabilityStatus::Partial),
            (FormatCapability::CamerasLights, CapabilityStatus::Partial),
            (
                FormatCapability::Animation,
                CapabilityStatus::IgnoredWithDiagnostic,
            ),
            (
                FormatCapability::Skinning,
                CapabilityStatus::Partial,
            ),
            (
                FormatCapability::MorphTargets,
                CapabilityStatus::IgnoredWithDiagnostic,
            ),
            (FormatCapability::Metadata, CapabilityStatus::Partial),
            (
                FormatCapability::CompressionContainers,
                CapabilityStatus::Partial,
            ),
            (
                FormatCapability::CoordinatesUnits,
                CapabilityStatus::Supported,
            ),
            (FormatCapability::Diagnostics, CapabilityStatus::Supported),
            (
                FormatCapability::ResourceLimits,
                CapabilityStatus::Supported,
            ),
        ])
        .with_notes("experimental glTF 2.0 importer for mesh, hierarchy, camera projection, skin MVP, PBR material factors, and texture URI references")
        .with_docs_path("docs/formats/gltf.md")
}

pub fn register(registry: &mut ImporterRegistry) -> Result<()> {
    registry.register(GltfImporter)
}

impl FormatImporter for GltfImporter {
    fn info(&self) -> FormatInfo {
        format_info()
    }

    fn can_read(&self, input: &mut dyn ReadSeek, hint: &ReadHint) -> Result<ReadConfidence> {
        detect::can_read(input, hint)
    }

    fn read(&self, ctx: &mut ImportContext<'_>) -> Result<Scene> {
        parser::read_gltf(ctx)
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
                (SupportMatrixColumn::Skinning, FormatCapability::Skinning),
                (
                    SupportMatrixColumn::CamerasLights,
                    FormatCapability::CamerasLights,
                ),
                (
                    SupportMatrixColumn::MorphTargets,
                    FormatCapability::MorphTargets,
                ),
                (
                    SupportMatrixColumn::ResourceLimits,
                    FormatCapability::ResourceLimits,
                ),
                (
                    SupportMatrixColumn::Diagnostics,
                    FormatCapability::Diagnostics,
                ),
            ],
        );
    }
}
