#![forbid(unsafe_code)]

mod detect;
mod error;
mod parser;

pub(crate) use error::PlyFailure;
pub use error::PlyFailureCode;

use baozi_import::authoring::FormatId;
use baozi_import::{
    CapabilityStatus, DetectionOutcome, DetectionResult, DetectionScope, FormatCapability,
    FormatDescriptor, FormatEncoding, FormatImporter, ParseResult, ParseScope, Probe, ReadHint,
    RegistryBuilder, RegistryRegistrationError, SceneDraft, TextBomPolicy, TextControlPolicy,
    TextDecodingMode, TextHardCaps, TextLexicalMode, TextNewline, TextProfile, TextSourceEncoding,
};

pub(crate) const FORMAT_ID: &str = "org.baozi.ply";

pub struct PlyImporter;

pub fn format_descriptor() -> FormatDescriptor {
    FormatDescriptor::builder(
        FormatId::new(FORMAT_ID).expect("built-in PLY format id is valid"),
        "PLY",
    )
    .extensions(["ply"])
    .media_types(["model/ply"])
    .encoding(FormatEncoding::TextOrBinary)
    .text_profile(text_profile())
    .capabilities([
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
    .notes("PLY importer for ASCII and binary vertex/face geometry")
    .docs_path("docs/formats/ply.md")
    .build()
    .expect("built-in PLY descriptor is valid")
}

fn text_profile() -> TextProfile {
    TextProfile::builder(
        TextHardCaps::new(
            512 * 1024 * 1024,
            512 * 1024 * 1024,
            64 * 1024 * 1024,
            1_000_000_000,
            64 * 1024 * 1024,
        )
        .expect("static PLY text hard caps must be valid"),
    )
    .source_encodings([TextSourceEncoding::Utf8])
    .bom_policy(TextBomPolicy::Forbidden)
    .newlines([TextNewline::Lf, TextNewline::CrLf])
    .lexical_mode(TextLexicalMode::AsciiOnly)
    .decoding_modes([TextDecodingMode::RejectInvalid])
    .control_policy(TextControlPolicy::AllowHorizontalTab)
    .build()
    .expect("static PLY text profile must be valid")
}

#[allow(
    clippy::result_large_err,
    reason = "the shared registry contract owns the registration error representation"
)]
pub fn register(registry: &mut RegistryBuilder) -> Result<(), RegistryRegistrationError> {
    registry.register(PlyImporter)
}

impl FormatImporter for PlyImporter {
    fn descriptor(&self) -> FormatDescriptor {
        format_descriptor()
    }

    fn detect(
        &self,
        probe: &Probe<'_>,
        hint: &ReadHint,
        scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        detect::detect(probe, hint, scope)
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        parser::read_ply(scope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baozi_test_support::assert_support_matrix_row;

    #[test]
    fn uses_qualified_stable_identity() {
        assert_eq!(format_descriptor().id().as_str(), "org.baozi.ply");
    }

    #[test]
    fn support_matrix_matches_format_descriptor() {
        assert_support_matrix_row("baozi-format-ply", &format_descriptor());
    }

    #[test]
    fn text_profile_matches_the_ascii_ply_baseline() {
        let descriptor = format_descriptor();
        let profile = descriptor
            .text_profile()
            .expect("text-or-binary PLY must declare a text profile");

        assert_eq!(
            profile.source_encodings().collect::<Vec<_>>(),
            [TextSourceEncoding::Utf8]
        );
        assert_eq!(profile.bom_policy(), TextBomPolicy::Forbidden);
        assert_eq!(
            profile.newlines().collect::<Vec<_>>(),
            [TextNewline::Lf, TextNewline::CrLf]
        );
        assert_eq!(profile.lexical_mode(), TextLexicalMode::AsciiOnly);
        assert_eq!(
            profile.decoding_modes().collect::<Vec<_>>(),
            [TextDecodingMode::RejectInvalid]
        );
        assert_eq!(
            profile.control_policy(),
            TextControlPolicy::AllowHorizontalTab
        );
        assert!(!profile.allows_decoding_mode(TextDecodingMode::ReplaceInvalidUtf8));

        let hard_caps = profile.hard_caps();
        assert_eq!(
            (
                hard_caps.max_raw_bytes(),
                hard_caps.max_decoded_utf8_bytes(),
                hard_caps.max_line_index_bytes(),
                hard_caps.max_tokens(),
                hard_caps.max_canonical_string_bytes(),
            ),
            (
                512 * 1024 * 1024,
                512 * 1024 * 1024,
                64 * 1024 * 1024,
                1_000_000_000,
                64 * 1024 * 1024,
            )
        );
    }
}
