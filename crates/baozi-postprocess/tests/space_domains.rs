use baozi_core::{
    AttributeData, AttributeDomain, AttributeSemanticRef, Axis, FormatId, FrontFaceWinding,
    Handedness, TargetSpace, ValidatedScene,
};
use baozi_import::authoring::{
    AttributeSemantic, AuthoredTransform, AuthoringRange, CustomAttributeSemantic,
    GeometryAttributeAuthoring, GeometryLogicalMeshAuthoring, GeometryPrimitiveAuthoring,
    GeometryTopologyAuthoring, NodeAuthoringNode, NodeLocalId, Presence, Quat, SceneSpace,
    StaticSceneBuilder, TangentBasisConvention, TangentHandednessEncoding, UvOrigin, Vec3,
};
use baozi_import::{
    CapabilityStatus, DetectionOutcome, DetectionResult, DetectionScope, FormatCapability,
    FormatDescriptor, FormatEncoding, FormatImporter, FormatSelection, ImportConfigOverlay,
    ImportOperationControl, ImportRequest, ImportRequestLayer, OptionCompileLimits, ParseResult,
    ParseScope, ReadConfidence, ReadHint, RegistryBuildLimits, RegistryBuilder, SceneDraft,
    import_bytes,
};
use baozi_io::AssetPath;
use baozi_postprocess::{
    PostProcessControl, PostProcessRunError, SpaceConversionCause, SpaceConversionIssue,
    SpaceConversionOptions, convert_scene_space,
};

#[derive(Clone, Copy)]
enum Fixture {
    Supported,
    Custom,
    UnknownEncoding,
}

struct DomainImporter(Fixture);

impl FormatImporter for DomainImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("org.baozi.space-domain").unwrap(),
            "Space domain fixture",
        )
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
        .build()
        .unwrap()
    }
    fn detect(
        &self,
        _: &baozi_import::Probe<'_>,
        _: &ReadHint,
        _: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }
    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        let positions = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let normals = [1.0, 2.0, 3.0, 1.0, 2.0, 3.0, 1.0, 2.0, 3.0];
        let tangents = [1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 1.0, 1.0];
        let mut attributes = vec![
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                &positions,
                Presence::Absent,
            ),
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Normal,
                AttributeDomain::Vertex,
                3,
                &normals,
                Presence::Absent,
            ),
            GeometryAttributeAuthoring::sparse_f32(
                AttributeSemantic::Normal,
                AttributeDomain::Corner,
                3,
                3,
                &[1, 2],
                &[0.0, 2.0, 0.0, 0.0, 0.0, 3.0],
                &[1.0, 0.0, 0.0],
            ),
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Tangent,
                AttributeDomain::Vertex,
                4,
                &tangents,
                Presence::Absent,
            ),
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Tangent,
                AttributeDomain::Corner,
                4,
                &tangents,
                Presence::Absent,
            ),
        ];
        let custom = [f64::from_bits(0x3ff0_0000_0000_0001), -0.0, 7.0];
        if matches!(self.0, Fixture::Custom) {
            attributes.push(GeometryAttributeAuthoring::f64(
                AttributeSemantic::Custom(CustomAttributeSemantic::new("test:precise").unwrap()),
                AttributeDomain::Corner,
                1,
                &custom,
            ));
        }
        let primitives = [GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Absent,
            Presence::Absent,
            AuthoringRange::new(0, attributes.len() as u32),
        )];
        let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
        let nodes = [NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::try_trs(
                Vec3::new(1.0, 2.0, 3.0),
                Quat::IDENTITY,
                Vec3::new(-2.0, 0.0, 4.0),
            )
            .unwrap(),
            Presence::Absent,
            Presence::Absent,
        )
        .with_name("root")];
        let space = if matches!(self.0, Fixture::UnknownEncoding) {
            SceneSpace::STANDARD_TARGET
        } else {
            SceneSpace {
                uv_origin: UvOrigin::LowerLeft,
                tangent_basis: TangentBasisConvention::MikkTspace,
                tangent_handedness_encoding: TangentHandednessEncoding::SignInW,
                ..SceneSpace::STANDARD_TARGET
            }
        };
        scope.submit_static_scene(
            StaticSceneBuilder::new(Presence::Present(&[NodeLocalId::new(0)]), &nodes)
                .with_scene_space(&space)
                .with_geometry(&meshes, &primitives, &attributes, &[]),
        )
    }
}

fn scene(fixture: Fixture) -> ValidatedScene {
    let mut registry = RegistryBuilder::new();
    registry.register(DomainImporter(fixture)).unwrap();
    let registry = registry.freeze(RegistryBuildLimits::default()).unwrap();
    let request = ImportRequest::new().with_caller_layer(ImportRequestLayer::new(
        ImportConfigOverlay::default().with_format_selection(FormatSelection::Forced(
            FormatId::new("org.baozi.space-domain").unwrap(),
        )),
    ));
    let request = registry
        .compile_request(&request, OptionCompileLimits::default())
        .unwrap();
    import_bytes(
        &request,
        AssetPath::new("domain.fixture").unwrap(),
        b"input",
        ImportOperationControl::default(),
    )
    .unwrap()
    .into_parts()
    .0
}

fn reflection() -> SpaceConversionOptions {
    SpaceConversionOptions {
        target: TargetSpace::try_new(
            Handedness::Left,
            Axis::PositiveY,
            Axis::NegativeZ,
            0.01,
            FrontFaceWinding::CounterClockwise,
        )
        .unwrap(),
        source_assumptions: None,
    }
}

#[test]
fn conversion_transports_all_normal_tangent_domains_and_sparse_explicitness() {
    let (scene, report) = convert_scene_space(
        scene(Fixture::Supported),
        reflection(),
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    let output = scene.meshes().next().unwrap().primitives().next().unwrap();
    assert_eq!(output.indices().unwrap(), [0, 2, 1]);
    let AttributeData::DenseF32 { values, lanes: 3 } =
        output.normals(AttributeDomain::Vertex).unwrap().data()
    else {
        panic!("vertex normals");
    };
    assert_eq!(values, [-1.0, 2.0, 3.0, -1.0, 2.0, 3.0, -1.0, 2.0, 3.0]);
    let AttributeData::SparseF32 {
        indices,
        values,
        default,
        ..
    } = output.normals(AttributeDomain::Corner).unwrap().data()
    else {
        panic!("sparse corner normals");
    };
    assert_eq!(indices, [1, 2]);
    assert_eq!(values, [0.0, 0.0, 3.0, 0.0, 2.0, 0.0]);
    assert_eq!(default, [-1.0, 0.0, 0.0]);
    for (domain, expected) in [
        (
            AttributeDomain::Vertex,
            [
                -1.0, 0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, -1.0,
            ],
        ),
        (
            AttributeDomain::Corner,
            [
                -1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 1.0, -1.0, 0.0, 1.0, 0.0, 1.0,
            ],
        ),
    ] {
        let AttributeData::DenseF32 { values, lanes: 4 } = output
            .attribute(AttributeSemanticRef::Tangent, domain)
            .unwrap()
            .data()
        else {
            panic!("tangents");
        };
        assert_eq!(values, expected);
    }
    let root = scene.roots().next().unwrap();
    assert_eq!(root.name(), Some("root"));
    let trs = root.authored_transform().authored_trs().unwrap();
    assert_eq!(trs.translation(), Vec3::new(-100.0, 200.0, 300.0));
    assert_eq!(trs.scale(), Vec3::new(-2.0, 0.0, 4.0));
    assert_eq!(scene.space().uv_origin, UvOrigin::LowerLeft);
    assert_eq!(
        report.space_conversion_effects().unwrap().source(),
        TargetSpace::STANDARD
    );
}

#[test]
fn unsupported_present_semantics_return_typed_locations_without_a_new_generation() {
    for (fixture, expected) in [
        (
            Fixture::Custom,
            SpaceConversionIssue::CustomAttribute { attribute: 5 },
        ),
        (
            Fixture::UnknownEncoding,
            SpaceConversionIssue::TangentEncoding { attribute: 3 },
        ),
    ] {
        let scene = scene(fixture);
        let generation = scene.generation();
        let failure =
            convert_scene_space(scene, reflection(), PostProcessControl::default()).unwrap_err();
        assert_eq!(failure.input().generation(), generation);
        let PostProcessRunError::SpaceConversion { failure } = failure.error() else {
            panic!("space failure");
        };
        assert!(
            matches!(failure.cause(), SpaceConversionCause::Issue { issue } if *issue == expected)
        );
        assert_eq!(failure.resources().unwrap().committed_canonical_bytes(), 0);
        assert_eq!(failure.resources().unwrap().current_live_bytes(), 0);
    }
}

#[test]
fn winding_only_conversion_preserves_custom_precision_and_does_not_flip_tangent_sign() {
    let target = TargetSpace::try_new(
        Handedness::Right,
        Axis::PositiveY,
        Axis::NegativeZ,
        1.0,
        FrontFaceWinding::Clockwise,
    )
    .unwrap();
    let (scene, report) = convert_scene_space(
        scene(Fixture::Custom),
        SpaceConversionOptions {
            target,
            source_assumptions: None,
        },
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    assert!(
        report
            .space_conversion_effects()
            .unwrap()
            .reverses_triangle_winding()
    );
    let output = scene.meshes().next().unwrap().primitives().next().unwrap();
    let custom = output
        .attributes()
        .find(|attribute| matches!(attribute.semantic(), AttributeSemanticRef::Custom(_)))
        .unwrap();
    let AttributeData::DenseF64 { values, lanes: 1 } = custom.data() else {
        panic!("precise custom tuples");
    };
    assert_eq!(
        values
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        [
            0x3ff0_0000_0000_0001,
            7.0_f64.to_bits(),
            (-0.0_f64).to_bits()
        ]
    );
    let AttributeData::DenseF32 { values, lanes: 4 } = output
        .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Corner)
        .unwrap()
        .data()
    else {
        panic!("tangents");
    };
    assert_eq!(
        values
            .as_chunks::<4>()
            .0
            .iter()
            .map(|tuple| tuple[3])
            .collect::<Vec<_>>(),
        [1.0, 1.0, -1.0]
    );
}
