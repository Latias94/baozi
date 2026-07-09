#![forbid(unsafe_code)]

mod ascii;
mod binary;
mod detect;
mod parser;

use baozi_core::{Result, Scene};
use baozi_import::{
    CapabilityStatus, FormatCapability, FormatEncoding, FormatImporter, FormatInfo, FormatMaturity,
    FormatSidecarPolicy, ImportContext, ImporterRegistry, ReadConfidence, ReadHint,
};
use baozi_io::ReadSeek;

pub struct StlImporter;

pub fn format_info() -> FormatInfo {
    FormatInfo::new("stl", "STL", &["stl"])
        .with_media_types(&["model/stl", "application/sla"])
        .with_encoding(FormatEncoding::TextOrBinary)
        .with_sidecar_policy(FormatSidecarPolicy::None)
        .with_maturity(FormatMaturity::Experimental)
        .with_capabilities(&[
            (FormatCapability::Geometry, CapabilityStatus::Supported),
            (FormatCapability::Hierarchy, CapabilityStatus::Partial),
            (FormatCapability::Materials, CapabilityStatus::Partial),
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
                CapabilityStatus::ParsedLossy,
            ),
            (FormatCapability::Diagnostics, CapabilityStatus::Supported),
            (
                FormatCapability::ResourceLimits,
                CapabilityStatus::Supported,
            ),
        ])
        .with_notes("experimental STL importer for binary and ASCII triangle meshes")
        .with_docs_path("docs/formats/stl.md")
}

pub fn register(registry: &mut ImporterRegistry) -> Result<()> {
    registry.register(StlImporter)
}

impl FormatImporter for StlImporter {
    fn info(&self) -> FormatInfo {
        format_info()
    }

    fn can_read(&self, input: &mut dyn ReadSeek, hint: &ReadHint) -> Result<ReadConfidence> {
        detect::can_read(input, hint)
    }

    fn read(&self, ctx: &mut ImportContext<'_>) -> Result<Scene> {
        parser::read_stl(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baozi_test_support::{SupportMatrixColumn, assert_support_matrix_row};

    #[test]
    fn reports_experimental_maturity() {
        assert_eq!(format_info().maturity(), FormatMaturity::Experimental);
    }

    #[test]
    fn support_matrix_matches_format_info() {
        assert_support_matrix_row(
            "baozi-format-stl",
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
