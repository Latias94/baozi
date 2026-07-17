#![forbid(unsafe_code)]

mod detect;
mod error;
mod mesh_builder;
mod mtl;
mod obj;
mod options;
mod parser;

pub use error::{ObjFailure, ObjFailureCode};
pub use options::{ObjImportOptions, ObjTextDecodingMode, obj_import_option_key};

use baozi_core::{FormatId, SceneDraft};
use baozi_import::{
    CapabilityStatus, DetectionOutcome, DetectionResult, DetectionScope, FormatCapability,
    FormatDescriptor, FormatEncoding, FormatImporter, OptionRuntimeTypeKey, ParseResult,
    ParseScope, Probe, ReadHint, RegistryBuilder, RegistryRegistrationError, TextBomPolicy,
    TextControlPolicy, TextDecodingMode, TextHardCaps, TextLexicalMode, TextNewline, TextProfile,
    TextSourceEncoding,
};
use std::sync::Arc;

pub const FORMAT_ID: &str = "org.baozi.obj";
const MATERIAL_LIBRARY_ROLE: &str = "material-library";

pub struct ObjImporter;

pub fn format_descriptor() -> FormatDescriptor {
    FormatDescriptor::builder(
        FormatId::new(FORMAT_ID).expect("static OBJ format id must be valid"),
        "Wavefront OBJ",
    )
    .extensions(["obj"])
    .media_types(["model/obj", "text/plain"])
    .encoding(FormatEncoding::Text)
    .text_profile(obj_text_profile())
    .capabilities([
        (FormatCapability::Geometry, CapabilityStatus::Supported),
        (FormatCapability::Hierarchy, CapabilityStatus::Partial),
        (FormatCapability::Materials, CapabilityStatus::Partial),
        (FormatCapability::Textures, CapabilityStatus::Partial),
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
    .dependency_roles([(
        material_library_role(),
        baozi_import::DependencyRequirement::Optional,
    )])
    .notes("OBJ/MTL importer for static face meshes and external texture URI references")
    .docs_path("docs/formats/obj.md")
    .build()
    .expect("static OBJ descriptor must be valid")
}

pub(crate) fn obj_text_profile() -> TextProfile {
    TextProfile::builder(
        TextHardCaps::new(
            512 * 1024 * 1024,
            1024 * 1024 * 1024,
            64 * 1024 * 1024,
            1_000_000_000,
            64 * 1024 * 1024,
        )
        .expect("static OBJ text hard caps must be valid"),
    )
    .source_encodings([TextSourceEncoding::Utf8])
    .bom_policy(TextBomPolicy::Accepted)
    .newlines([TextNewline::Lf, TextNewline::CrLf])
    .lexical_mode(TextLexicalMode::AsciiSyntaxUtf8Values)
    .decoding_modes([
        TextDecodingMode::RejectInvalid,
        TextDecodingMode::ReplaceInvalidUtf8,
    ])
    .control_policy(TextControlPolicy::AllowHorizontalTab)
    .build()
    .expect("static OBJ text profile must be valid")
}

#[allow(
    clippy::result_large_err,
    reason = "registration errors remain allocation-free at this trusted configuration boundary"
)]
pub fn register(registry: &mut RegistryBuilder) -> Result<(), RegistryRegistrationError> {
    registry.register(ObjImporter)
}

pub(crate) fn material_library_role() -> baozi_import::DependencyRoleId {
    baozi_import::DependencyRoleId::new(MATERIAL_LIBRARY_ROLE)
        .expect("static OBJ dependency role must be valid")
}

impl FormatImporter for ObjImporter {
    fn descriptor(&self) -> FormatDescriptor {
        format_descriptor()
    }

    fn option_codec(&self) -> Arc<dyn baozi_import::FormatOptionCodec> {
        options::option_codec()
    }

    fn option_runtime_type_key(&self) -> OptionRuntimeTypeKey {
        OptionRuntimeTypeKey::of::<ObjImportOptions>()
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
        parser::read_obj(scope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baozi_test_support::assert_support_matrix_row;

    #[test]
    fn descriptor_uses_qualified_identity_and_optional_material_library() {
        let descriptor = format_descriptor();
        assert_eq!(descriptor.id().as_str(), FORMAT_ID);
        assert_eq!(
            descriptor
                .dependency_roles()
                .map(|(role, requirement)| (role.as_str(), requirement))
                .collect::<Vec<_>>(),
            vec![(
                MATERIAL_LIBRARY_ROLE,
                baozi_import::DependencyRequirement::Optional
            )]
        );
    }

    #[test]
    fn support_matrix_matches_format_descriptor() {
        assert_support_matrix_row("baozi-format-obj", &format_descriptor());
    }
}
