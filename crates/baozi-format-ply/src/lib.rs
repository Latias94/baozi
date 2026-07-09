#![forbid(unsafe_code)]

use baozi_core::{BaoziError, Result, Scene};
use baozi_import::{
    CapabilityStatus, FormatCapability, FormatEncoding, FormatImporter, FormatInfo, FormatMaturity,
    FormatSidecarPolicy, ImportContext, ImporterRegistry,
};

pub struct PlyImporter;

pub fn format_info() -> FormatInfo {
    FormatInfo::new("ply", "PLY", &["ply"])
        .with_media_types(&["model/ply"])
        .with_encoding(FormatEncoding::TextOrBinary)
        .with_sidecar_policy(FormatSidecarPolicy::None)
        .with_maturity(FormatMaturity::Experimental)
        .with_capabilities(&[
            (FormatCapability::Geometry, CapabilityStatus::Unknown),
            (FormatCapability::Hierarchy, CapabilityStatus::Unknown),
            (FormatCapability::Materials, CapabilityStatus::Unknown),
            (FormatCapability::Textures, CapabilityStatus::Unsupported),
            (
                FormatCapability::CamerasLights,
                CapabilityStatus::Unsupported,
            ),
            (FormatCapability::Animation, CapabilityStatus::Unsupported),
            (FormatCapability::Skinning, CapabilityStatus::Unsupported),
            (
                FormatCapability::MorphTargets,
                CapabilityStatus::Unsupported,
            ),
            (FormatCapability::Metadata, CapabilityStatus::Unknown),
            (
                FormatCapability::CompressionContainers,
                CapabilityStatus::Unsupported,
            ),
            (
                FormatCapability::CoordinatesUnits,
                CapabilityStatus::Unknown,
            ),
            (FormatCapability::Diagnostics, CapabilityStatus::Unknown),
            (FormatCapability::ResourceLimits, CapabilityStatus::Unknown),
        ])
        .with_notes("planned PLY importer shell; parsing is not implemented yet")
        .with_docs_path("docs/formats/ply.md")
}

pub fn register(registry: &mut ImporterRegistry) -> Result<()> {
    registry.register(PlyImporter)
}

impl FormatImporter for PlyImporter {
    fn info(&self) -> FormatInfo {
        format_info()
    }

    fn read(&self, _ctx: &mut ImportContext<'_>) -> Result<Scene> {
        Err(BaoziError::unsupported_format("ply parser not implemented"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baozi_test_support::{SupportMatrixColumn, assert_support_matrix_row};

    #[test]
    fn support_matrix_matches_format_info() {
        assert_support_matrix_row(
            "baozi-format-ply",
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
