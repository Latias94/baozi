#![forbid(unsafe_code)]

mod detect;
mod error;
mod parser;

pub(crate) use error::GltfFailure;
pub use error::GltfFailureCode;

use baozi_core::{FormatId, SceneDraft};
use baozi_import::{
    CapabilityStatus, DependencyRequirement, DependencyRoleId, DetectionOutcome, DetectionResult,
    DetectionScope, FormatCapability, FormatDescriptor, FormatEncoding, FormatImporter,
    ParseResult, ParseScope, Probe, ReadHint, TextBomPolicy, TextControlPolicy, TextDecodingMode,
    TextHardCaps, TextLexicalMode, TextNewline, TextProfile, TextSourceEncoding,
};

/// Stable identity used only by the internal development harness.
pub const FORMAT_ID: &str = "org.baozi.gltf";
const EXTERNAL_BUFFER_ROLE: &str = "external-buffer";

/// Internal importer retained outside the admitted facade registry until U6.
#[doc(hidden)]
pub struct GltfImporter;

/// Describes the experimental parser without admitting it to public format sets.
#[doc(hidden)]
pub fn format_descriptor_for_development() -> FormatDescriptor {
    FormatDescriptor::builder(
        FormatId::new(FORMAT_ID).expect("static glTF format id must be valid"),
        "glTF 2.0",
    )
    .extensions(["glb", "gltf"])
    .media_types(["model/gltf+json", "model/gltf-binary"])
    .encoding(FormatEncoding::TextOrBinary)
    .text_profile(text_profile())
    .capabilities([
        (FormatCapability::Geometry, CapabilityStatus::Partial),
        (FormatCapability::Hierarchy, CapabilityStatus::Supported),
        (FormatCapability::Materials, CapabilityStatus::Partial),
        (FormatCapability::Textures, CapabilityStatus::ParsedLossy),
        (FormatCapability::CamerasLights, CapabilityStatus::Partial),
        (FormatCapability::Animation, CapabilityStatus::Unsupported),
        (FormatCapability::Skinning, CapabilityStatus::Partial),
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
            CapabilityStatus::Supported,
        ),
        (FormatCapability::Diagnostics, CapabilityStatus::Supported),
        (FormatCapability::ResourceLimits, CapabilityStatus::Partial),
    ])
    .dependency_roles([(external_buffer_role(), DependencyRequirement::Required)])
    .notes("internal glTF 2.0 development importer; not admitted to facade registries")
    .docs_path("docs/formats/gltf.md")
    .build()
    .expect("static glTF descriptor must be valid")
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
        .expect("static glTF text hard caps must be valid"),
    )
    .source_encodings([TextSourceEncoding::Utf8])
    .bom_policy(TextBomPolicy::Forbidden)
    .newlines([TextNewline::Lf, TextNewline::CrLf, TextNewline::Cr])
    .lexical_mode(TextLexicalMode::Utf8Document)
    .decoding_modes([TextDecodingMode::RejectInvalid])
    .control_policy(TextControlPolicy::AllowHorizontalTab)
    .build()
    .expect("static glTF text profile must be valid")
}

pub(crate) fn external_buffer_role() -> DependencyRoleId {
    DependencyRoleId::new(EXTERNAL_BUFFER_ROLE).expect("static glTF dependency role must be valid")
}

impl FormatImporter for GltfImporter {
    fn descriptor(&self) -> FormatDescriptor {
        format_descriptor_for_development()
    }

    fn detect(
        &self,
        probe: &Probe<'_>,
        hint: &ReadHint,
        scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        detect::can_read(probe, hint, scope)
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        parser::read_gltf(scope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_is_qualified_and_external_buffers_are_required() {
        let descriptor = format_descriptor_for_development();
        assert_eq!(descriptor.id().as_str(), FORMAT_ID);
        assert!(descriptor.dependency_roles().any(|(role, requirement)| {
            role == &external_buffer_role() && requirement == DependencyRequirement::Required
        }));
        assert_eq!(
            descriptor.capability_status(FormatCapability::ResourceLimits),
            Some(CapabilityStatus::Partial)
        );
    }

    #[test]
    fn text_profile_matches_the_strict_gltf_baseline() {
        let descriptor = format_descriptor_for_development();
        let profile = descriptor
            .text_profile()
            .expect("text-or-binary glTF must declare a text profile");

        assert_eq!(
            profile.source_encodings().collect::<Vec<_>>(),
            [TextSourceEncoding::Utf8]
        );
        assert_eq!(profile.bom_policy(), TextBomPolicy::Forbidden);
        assert_eq!(
            profile.newlines().collect::<Vec<_>>(),
            [TextNewline::Lf, TextNewline::CrLf, TextNewline::Cr]
        );
        assert_eq!(profile.lexical_mode(), TextLexicalMode::Utf8Document);
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
