#![forbid(unsafe_code)]

mod admission;
mod detect;
mod error;
mod json_syntax;
mod parser;
mod source_map;

pub(crate) use error::GltfFailure;
pub use error::GltfFailureCode;

use baozi_import::authoring::FormatId;
use baozi_import::{
    CapabilityStatus, DependencyRequirement, DependencyRoleId, DetectionOutcome, DetectionResult,
    DetectionScope, FormatCapability, FormatDescriptor, FormatEncoding, FormatImporter,
    ParseResult, ParseScope, Probe, ReadHint, RegistryBuilder, RegistryRegistrationError,
    SceneDraft, TextBomPolicy, TextControlPolicy, TextDecodingMode, TextHardCaps, TextLexicalMode,
    TextNewline, TextProfile, TextSourceEncoding,
};

/// Stable identity of the selected glTF 2.0 and GLB profile.
pub const FORMAT_ID: &str = "org.baozi.gltf";
const EXTERNAL_BUFFER_ROLE: &str = "external-buffer";
const EXTERNAL_IMAGE_ROLE: &str = "external-image";

/// Imports the selected glTF 2.0 and GLB profile documented in `docs/formats/gltf.md`.
pub struct GltfImporter;

/// Describes the supported glTF profile and its explicit partial capabilities.
pub fn format_descriptor() -> FormatDescriptor {
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
        (FormatCapability::Animation, CapabilityStatus::Partial),
        (FormatCapability::Skinning, CapabilityStatus::Partial),
        (FormatCapability::MorphTargets, CapabilityStatus::Partial),
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
    .dependency_roles([
        (external_buffer_role(), DependencyRequirement::Required),
        (external_image_role(), DependencyRequirement::Required),
    ])
    .notes("selected-scene glTF 2.0/GLB PBR, skin, morph, and TRS animation profile; see exact restrictions")
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

pub(crate) fn external_image_role() -> DependencyRoleId {
    DependencyRoleId::new(EXTERNAL_IMAGE_ROLE).expect("static glTF dependency role must be valid")
}

impl FormatImporter for GltfImporter {
    fn descriptor(&self) -> FormatDescriptor {
        format_descriptor()
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

/// Registers the selected glTF profile in a caller-owned registry proposal.
#[allow(
    clippy::result_large_err,
    reason = "the shared registry contract owns the registration error representation"
)]
pub fn register(registry: &mut RegistryBuilder) -> Result<(), RegistryRegistrationError> {
    registry.register(GltfImporter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_is_qualified_and_external_resources_are_required() {
        let descriptor = format_descriptor();
        assert_eq!(descriptor.id().as_str(), FORMAT_ID);
        assert!(descriptor.dependency_roles().any(|(role, requirement)| {
            role == &external_buffer_role() && requirement == DependencyRequirement::Required
        }));
        assert!(descriptor.dependency_roles().any(|(role, requirement)| {
            role == &external_image_role() && requirement == DependencyRequirement::Required
        }));
        assert_eq!(
            descriptor.capability_status(FormatCapability::ResourceLimits),
            Some(CapabilityStatus::Partial)
        );
        assert_eq!(
            descriptor.capability_status(FormatCapability::Animation),
            Some(CapabilityStatus::Partial)
        );
    }

    #[test]
    fn text_profile_matches_the_strict_gltf_baseline() {
        let descriptor = format_descriptor();
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

    #[test]
    fn support_matrix_matches_format_descriptor() {
        baozi_test_support::assert_support_matrix_row("baozi-format-gltf", &format_descriptor());
    }
}
