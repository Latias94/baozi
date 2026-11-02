#![forbid(unsafe_code)]

mod detect;
mod error;
mod mesh_builder;
mod mtl;
mod obj;
mod options;
mod parser;

pub(crate) use error::ObjFailure;
pub use error::ObjFailureCode;
pub use options::{ObjImportOptions, ObjTextDecodingMode};

use baozi_import::authoring::FormatId;
use baozi_import::{
    CapabilityStatus, CompiledOptions, DetectionOutcome, DetectionResult, DetectionScope, Format,
    FormatCapability, FormatDescriptor, FormatEncoding, FormatOptionLayers, FormatOptionSchema,
    OptionCompileError, OptionCompileScope, ParseContext, ParseResult, Probe, ReadHint,
    RegistryBuildScope, RegistryBuilder, RegistryRegistrationError, TextBomPolicy,
    TextControlPolicy, TextDecodingMode, TextHardCaps, TextLexicalMode, TextNewline, TextProfile,
    TextSourceEncoding,
};

pub const FORMAT_ID: &str = "org.baozi.obj";
const DIFFUSE_TEXTURE_ROLE: &str = "diffuse-texture";
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
    .dependency_roles([
        (
            diffuse_texture_role(),
            baozi_import::DependencyRequirement::Optional,
        ),
        (
            material_library_role(),
            baozi_import::DependencyRequirement::Optional,
        ),
    ])
    .notes("OBJ/MTL importer for static face meshes and session-snapshotted diffuse textures")
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

pub(crate) fn diffuse_texture_role() -> baozi_import::DependencyRoleId {
    baozi_import::DependencyRoleId::new(DIFFUSE_TEXTURE_ROLE)
        .expect("static OBJ dependency role must be valid")
}

impl Format for ObjImporter {
    type Options = ObjImportOptions;

    fn descriptor(&self) -> FormatDescriptor {
        format_descriptor()
    }

    fn option_schema(&self) -> Option<FormatOptionSchema> {
        Some(options::option_schema())
    }

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledOptions<Self::Options>, OptionCompileError> {
        options::compile_defaults(scope)
    }

    fn compile_options(
        &self,
        scope: &mut OptionCompileScope<'_>,
        defaults: &Self::Options,
        layers: FormatOptionLayers<'_>,
    ) -> Result<CompiledOptions<Self::Options>, OptionCompileError> {
        options::compile_options(scope, defaults, layers)
    }

    fn detect(
        &self,
        probe: &Probe<'_>,
        hint: &ReadHint,
        scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        detect::can_read(probe, hint, scope)
    }

    fn parse(&self, options: &Self::Options, context: &mut ParseContext<'_>) -> ParseResult<()> {
        parser::read_obj(options, context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baozi_import::{
        FormatSelection, ImportConfigOverlay, ImportOperationControl, ImportRequest,
        ImportRequestLayer, OptionCompileLimits, RegistryBuildLimits,
    };
    use baozi_io::AssetPath;
    use baozi_test_support::assert_support_matrix_row;

    #[test]
    fn descriptor_uses_qualified_identity_and_optional_dependencies() {
        let descriptor = format_descriptor();
        assert_eq!(descriptor.id().as_str(), FORMAT_ID);
        assert_eq!(
            descriptor
                .dependency_roles()
                .map(|(role, requirement)| (role.as_str(), requirement))
                .collect::<Vec<_>>(),
            vec![
                (
                    DIFFUSE_TEXTURE_ROLE,
                    baozi_import::DependencyRequirement::Optional
                ),
                (
                    MATERIAL_LIBRARY_ROLE,
                    baozi_import::DependencyRequirement::Optional
                ),
            ]
        );
    }

    #[test]
    fn support_matrix_matches_format_descriptor() {
        assert_support_matrix_row("baozi-format-obj", &format_descriptor());
    }

    #[test]
    fn complete_batch_path_imports_triangle() {
        let mut registry = RegistryBuilder::new();
        register(&mut registry).expect("OBJ registration");
        let registry = registry
            .freeze(RegistryBuildLimits::default())
            .expect("OBJ registry freeze");
        let config = ImportConfigOverlay::default().with_format_selection(FormatSelection::Forced(
            FormatId::new(FORMAT_ID).expect("static OBJ format id"),
        ));
        let request = ImportRequest::new().with_caller_layer(ImportRequestLayer::new(config));
        let request = registry
            .plan(&request, OptionCompileLimits::default())
            .expect("OBJ request compilation");
        let report = request
            .read_bytes(
                AssetPath::new("triangle.obj").expect("test source path"),
                b"o triangle\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
                ImportOperationControl::default(),
            )
            .expect("complete-batch OBJ import");

        assert!(report.diagnostics().is_empty());
        assert!(report.scene().canonical_block_count() > 0);
    }
}
