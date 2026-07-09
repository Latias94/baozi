#![forbid(unsafe_code)]

mod detect;
mod parser;

use baozi_core::{Result, Scene};
use baozi_import::{
    CapabilityStatus, FormatCapability, FormatEncoding, FormatImporter, FormatInfo, FormatMaturity,
    FormatSidecarPolicy, ImportContext, ImporterRegistry, ReadConfidence, ReadHint,
};
use baozi_io::ReadSeek;

pub struct PlyImporter;

pub fn format_info() -> FormatInfo {
    FormatInfo::new("ply", "PLY", &["ply"])
        .with_media_types(&["model/ply"])
        .with_encoding(FormatEncoding::TextOrBinary)
        .with_sidecar_policy(FormatSidecarPolicy::None)
        .with_maturity(FormatMaturity::Experimental)
        .with_capabilities(&[
            (FormatCapability::Geometry, CapabilityStatus::Supported),
            (FormatCapability::Hierarchy, CapabilityStatus::Partial),
            (FormatCapability::Materials, CapabilityStatus::Unsupported),
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
            (FormatCapability::Metadata, CapabilityStatus::Partial),
            (
                FormatCapability::CompressionContainers,
                CapabilityStatus::Unsupported,
            ),
            (
                FormatCapability::CoordinatesUnits,
                CapabilityStatus::Unknown,
            ),
            (FormatCapability::Diagnostics, CapabilityStatus::Supported),
            (
                FormatCapability::ResourceLimits,
                CapabilityStatus::Supported,
            ),
        ])
        .with_notes("experimental PLY importer for ASCII and binary vertex/face geometry")
        .with_docs_path("docs/formats/ply.md")
}

pub fn register(registry: &mut ImporterRegistry) -> Result<()> {
    registry.register(PlyImporter)
}

impl FormatImporter for PlyImporter {
    fn info(&self) -> FormatInfo {
        format_info()
    }

    fn can_read(&self, input: &mut dyn ReadSeek, hint: &ReadHint) -> Result<ReadConfidence> {
        detect::can_read(input, hint)
    }

    fn read(&self, ctx: &mut ImportContext<'_>) -> Result<Scene> {
        parser::read_ply(ctx)
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
