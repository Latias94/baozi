use baozi_core::{AttributeDomain, AttributeSemanticRef, FormatId};
use baozi_import::authoring::{
    AttributeSemantic, AuthoringRange, GeometryAttributeAuthoring, GeometryLogicalMeshAuthoring,
    GeometryPrimitiveAuthoring, GeometryTopologyAuthoring, Presence, StaticSceneBuilder,
};
use baozi_import::{
    CapabilityStatus, CompiledOptions, DetectionOutcome, DetectionResult, DetectionScope, Format,
    FormatCapability, FormatDescriptor, FormatEncoding, FormatSelection, ImportConfigOverlay,
    ImportOperationControl, ImportRequest, ImportRequestLayer, NoFormatOptions, OptionCompileError,
    OptionCompileLimits, ParseContext, ParseResult, ReadConfidence, ReadHint, RegistryBuildLimits,
    RegistryBuildScope, RegistryBuilder,
};
use baozi_io::AssetPath;
use baozi_postprocess::{
    ExistingNormals, NormalCause, NormalOptions, PostProcessControl, PostProcessRunError,
    generate_normals,
};

#[derive(Clone, Copy)]
enum MultipleFields {
    Normals,
    Tangents,
}

struct DomainImporter(MultipleFields);

impl Format for DomainImporter {
    type Options = NoFormatOptions;

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledOptions<Self::Options>, OptionCompileError> {
        NoFormatOptions::compile_defaults(scope)
    }

    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("org.baozi.multidomain").unwrap(),
            "Domain fixture",
        )
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
        .build()
        .unwrap()
    }

    fn detect(
        &self,
        _probe: &baozi_import::Probe<'_>,
        _hint: &ReadHint,
        _scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, _options: &Self::Options, context: &mut ParseContext<'_>) -> ParseResult<()> {
        let positions = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let normal = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
        let tangent = [1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0];
        let (semantic, lanes, values) = match self.0 {
            MultipleFields::Normals => (AttributeSemantic::Normal, 3, normal.as_slice()),
            MultipleFields::Tangents => (AttributeSemantic::Tangent, 4, tangent.as_slice()),
        };
        let attributes = [
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                &positions,
                Presence::Absent,
            ),
            GeometryAttributeAuthoring::f32(
                semantic.clone(),
                AttributeDomain::Vertex,
                lanes,
                values,
                Presence::Absent,
            ),
            GeometryAttributeAuthoring::f32(
                semantic,
                AttributeDomain::Corner,
                lanes,
                values,
                Presence::Absent,
            ),
        ];
        let primitives = [GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Absent,
            baozi_core::MaterialAssignment::Unspecified,
            AuthoringRange::new(0, 3),
        )];
        let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
        context.submit(
            StaticSceneBuilder::new(Presence::Present(&[]), &[]).with_geometry(
                &meshes,
                &primitives,
                &attributes,
                &[],
            ),
        )
    }
}

#[test]
fn valid_multiple_attribute_domains_fail_with_a_capability_error_before_replacement() {
    for fields in [MultipleFields::Normals, MultipleFields::Tangents] {
        let mut registry = RegistryBuilder::new();
        registry.register(DomainImporter(fields)).unwrap();
        let registry = registry.freeze(RegistryBuildLimits::default()).unwrap();
        let request = ImportRequest::new().with_caller_layer(ImportRequestLayer::new(
            ImportConfigOverlay::default().with_format_selection(FormatSelection::Forced(
                FormatId::new("org.baozi.multidomain").unwrap(),
            )),
        ));
        let request = registry
            .plan(&request, OptionCompileLimits::default())
            .unwrap();
        let (scene, evidence) = request
            .read_bytes(
                AssetPath::new("domain.fixture").unwrap(),
                b"input",
                ImportOperationControl::default(),
            )
            .expect("both domains are valid canonical input")
            .into_parts();
        let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
        let target = primitive.id();
        let semantic = match fields {
            MultipleFields::Normals => AttributeSemanticRef::Normal,
            MultipleFields::Tangents => AttributeSemanticRef::Tangent,
        };
        assert_eq!(
            primitive
                .attributes()
                .filter(|attribute| attribute.semantic() == semantic)
                .count(),
            2
        );
        let failure = generate_normals(
            scene,
            target,
            NormalOptions {
                existing: ExistingNormals::Replace,

                ..NormalOptions::default()
            },
            baozi_postprocess::TangentPolicy::Remove,
            PostProcessControl::default(),
        )
        .unwrap_err();
        assert!(evidence.matches_scene(failure.input()));
        assert_eq!(failure.input().generation(), target.generation());
        let PostProcessRunError::Normals { failure } = failure.error() else {
            panic!("normal failure");
        };
        assert!(matches!(
            (fields, failure.cause()),
            (MultipleFields::Normals, NormalCause::MultipleNormalFields)
                | (MultipleFields::Tangents, NormalCause::MultipleTangentFields)
        ));
        let resources = failure.resources().unwrap();
        assert_eq!(resources.current_live_bytes(), 0);
        assert_eq!(resources.active_reservations(), 0);
        assert_eq!(resources.committed_canonical_bytes(), 0);
    }
}
