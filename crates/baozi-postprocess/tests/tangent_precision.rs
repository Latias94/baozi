use baozi_core::{AttributeData, AttributeDomain, AttributeSemanticRef, FormatId, SceneSpace};
use baozi_import::authoring::{
    AttributeSemantic, AuthoringRange, GeometryAttributeAuthoring, GeometryLogicalMeshAuthoring,
    GeometryPrimitiveAuthoring, GeometryTopologyAuthoring, Presence, StaticSceneBuilder,
};
use baozi_import::{
    CapabilityStatus, CompiledOptions, DetectionOutcome, DetectionResult, DetectionScope, Format,
    FormatCapability, FormatDescriptor, FormatEncoding, FormatSelection, ImportConfigOverlay,
    ImportOperationControl, ImportReport, ImportRequest, ImportRequestLayer, NoFormatOptions,
    OptionCompileError, OptionCompileLimits, ParseContext, ParseResult, ReadConfidence, ReadHint,
    RegistryBuildLimits, RegistryBuildScope, RegistryBuilder,
};
use baozi_io::AssetPath;
use baozi_postprocess::{
    GeometryNumericIssue, PostProcessControl, PostProcessRunDisposition, PostProcessRunError,
    TangentCause, TangentIssue, TangentOptions, generate_tangents,
};

const FORMAT: &str = "dev.baozi.test.tangent-precision";

#[derive(Clone, Copy)]
struct Fixture {
    positions: [[f64; 3]; 3],
    uvs: [[f64; 2]; 3],
    position_f64: bool,
    normal_f64: bool,
    uv_f64: bool,
    sparse: bool,
    sparse_default: [f64; 3],
}

impl Fixture {
    const BASIC: Self = Self {
        positions: [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        uvs: [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        position_f64: false,
        normal_f64: false,
        uv_f64: false,
        sparse: false,
        sparse_default: [-0.0; 3],
    };
}

struct Importer(Fixture);

impl Format for Importer {
    type Options = NoFormatOptions;

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledOptions<Self::Options>, OptionCompileError> {
        NoFormatOptions::compile_defaults(scope)
    }

    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(FormatId::new(FORMAT).unwrap(), "Tangent precision fixture")
            .encoding(FormatEncoding::Binary)
            .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
            .build()
            .unwrap()
    }

    fn detect(
        &self,
        _: &baozi_import::Probe<'_>,
        _: &ReadHint,
        _: &mut DetectionScope<'_, '_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(
        &self,
        _options: &Self::Options,
        context: &mut ParseContext<'_, '_>,
    ) -> ParseResult<()> {
        let f = self.0;
        let narrow_positions = f.positions.map(|p| p.map(|v| v as f32));
        let narrow_uvs = f.uvs.map(|p| p.map(|v| v as f32));
        let normals = [[0.0_f64, 0.0, 1.0]; 3];
        let narrow_normals = [[0.0_f32, 0.0, 1.0]; 3];
        let position = if f.position_f64 {
            if f.sparse {
                GeometryAttributeAuthoring::sparse_f64(
                    AttributeSemantic::Position,
                    AttributeDomain::Vertex,
                    3,
                    10_000_000,
                    &[0, 1, 2],
                    f.positions.as_flattened(),
                    &f.sparse_default,
                )
            } else {
                GeometryAttributeAuthoring::f64(
                    AttributeSemantic::Position,
                    AttributeDomain::Vertex,
                    3,
                    f.positions.as_flattened(),
                )
            }
        } else {
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                narrow_positions.as_flattened(),
                Presence::Absent,
            )
        };
        let normal = if f.normal_f64 {
            // All corners deliberately use the sparse default tuple.
            GeometryAttributeAuthoring::sparse_f64(
                AttributeSemantic::Normal,
                AttributeDomain::Corner,
                3,
                3,
                &[],
                &[],
                &normals[0],
            )
        } else {
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Normal,
                AttributeDomain::Corner,
                3,
                narrow_normals.as_flattened(),
                Presence::Absent,
            )
        };
        let uv = if f.uv_f64 {
            GeometryAttributeAuthoring::sparse_f64(
                AttributeSemantic::Texcoord(0),
                AttributeDomain::Corner,
                2,
                3,
                &[1, 2],
                f.uvs[1..].as_flattened(),
                &f.uvs[0],
            )
        } else {
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Texcoord(0),
                AttributeDomain::Corner,
                2,
                narrow_uvs.as_flattened(),
                Presence::Absent,
            )
        };
        let attributes = [position, normal, uv];
        let primitives = [GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Present(&[0, 1, 2]),
            baozi_core::MaterialAssignment::Unspecified,
            AuthoringRange::new(0, 3),
        )];
        let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
        context.submit(
            StaticSceneBuilder::new(Presence::Present(&[]), &[])
                .with_scene_space(&SceneSpace::STANDARD_TARGET)
                .with_geometry(&meshes, &primitives, &attributes, &[]),
        )
    }
}

fn import(fixture: Fixture) -> ImportReport {
    let mut registry = RegistryBuilder::new();
    registry.register(Importer(fixture)).unwrap();
    let registry = registry.freeze(RegistryBuildLimits::default()).unwrap();
    let request = ImportRequest::new().with_caller_layer(ImportRequestLayer::new(
        ImportConfigOverlay::default()
            .with_format_selection(FormatSelection::Forced(FormatId::new(FORMAT).unwrap())),
    ));
    let request = registry
        .plan(&request, OptionCompileLimits::default())
        .unwrap();
    request
        .read_bytes(
            AssetPath::new("precision.fixture").unwrap(),
            b"precision",
            ImportOperationControl::default(),
        )
        .unwrap()
}

fn assert_basis(fixture: Fixture, expected: [f64; 4]) {
    let (scene, import_evidence) = import(fixture).into_parts();
    let target = scene
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap()
        .id();
    let output = generate_tangents(
        scene,
        target,
        TangentOptions::new(AttributeDomain::Corner, 0, AttributeDomain::Corner),
        PostProcessControl::default(),
    )
    .expect("ordinary declared F64 fields must generate a retained F64 tangent");
    let (scene, report) = output.into_parts();
    assert_eq!(report.disposition(), PostProcessRunDisposition::Changed);
    assert_eq!(report.resources().current_live_bytes(), 0);
    assert_eq!(report.resources().active_reservations(), 0);
    assert!(report.matches_scene(&scene));
    assert!(!import_evidence.matches_scene(&scene));
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    let tangent = primitive
        .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Corner)
        .unwrap();
    let AttributeData::DenseF64 { lanes: 4, values } = tangent.data() else {
        panic!("the widest dependency determines the generated scalar width");
    };
    for value in values.chunks_exact(4) {
        for (&actual, expected) in value.iter().zip(expected) {
            assert!((actual - expected).abs() <= 16.0 * f64::EPSILON);
        }
    }
    if fixture.position_f64 {
        let position = primitive.positions().unwrap();
        let retained = match position.data() {
            AttributeData::DenseF64 { values, .. } => values,
            AttributeData::SparseF64 {
                logical_count,
                values,
                default,
                ..
            } => {
                assert_eq!(logical_count, 10_000_000);
                assert!(
                    default
                        .iter()
                        .zip(fixture.sparse_default)
                        .all(|(actual, expected)| actual.to_bits() == expected.to_bits())
                );
                values
            }
            _ => panic!("copy-only Position values must retain their encoding"),
        };
        assert_eq!(
            retained.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            fixture
                .positions
                .as_flattened()
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn each_f64_dependency_and_sparse_default_selects_f64_output() {
    for fixture in [
        Fixture {
            position_f64: true,
            ..Fixture::BASIC
        },
        Fixture {
            normal_f64: true,
            ..Fixture::BASIC
        },
        Fixture {
            uv_f64: true,
            ..Fixture::BASIC
        },
        Fixture {
            position_f64: true,
            normal_f64: true,
            uv_f64: true,
            sparse: true,
            ..Fixture::BASIC
        },
    ] {
        assert_basis(fixture, [1.0, 0.0, 0.0, 1.0]);
    }
}

#[test]
fn adjacent_large_positions_and_uvs_do_not_project_through_f32() {
    let base = 1_099_511_627_776.0;
    assert_basis(
        Fixture {
            positions: [
                [base, base, -0.0],
                [base + 1.0, base, -0.0],
                [base, base + 1.0, -0.0],
            ],
            uvs: [[base, base], [base + 1.0, base], [base, base + 1.0]],
            position_f64: true,
            uv_f64: true,
            ..Fixture::BASIC
        },
        [1.0, 0.0, 0.0, 1.0],
    );
}

#[test]
fn uniformly_tiny_subnormal_and_huge_f64_positions_have_the_same_frame() {
    for scale in [f64::from_bits(1), 1.0e-250, 1.0e250, f64::MAX / 4.0] {
        assert_basis(
            Fixture {
                positions: [[0.0, 0.0, -0.0], [scale, 0.0, -0.0], [0.0, scale, -0.0]],
                position_f64: true,
                ..Fixture::BASIC
            },
            [1.0, 0.0, 0.0, 1.0],
        );
    }
}

#[test]
fn mirrored_f64_uvs_retain_analytic_handedness() {
    assert_basis(
        Fixture {
            uvs: [[0.0, 0.0], [-1.0, 0.0], [0.0, 1.0]],
            uv_f64: true,
            ..Fixture::BASIC
        },
        [-1.0, 0.0, 0.0, -1.0],
    );
}

#[test]
fn f64_normal_does_not_discard_the_f32_position_determinant_specialization() {
    let root_two = 2.0_f64.sqrt();
    assert_basis(
        Fixture {
            positions: [
                [2.0_f64.powi(-100), 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [2.0, 2.0, 0.0],
            ],
            normal_f64: true,
            ..Fixture::BASIC
        },
        [1.0 / root_two, 1.0 / root_two, 0.0, -1.0],
    );
}

#[test]
fn common_primitive_scale_includes_the_used_sparse_default() {
    let tiny = 2.0_f64.powi(-1000);
    let fixture = Fixture {
        positions: [[0.0; 3], [tiny, 0.0, 0.0], [0.0, tiny, 0.0]],
        position_f64: true,
        sparse: true,
        sparse_default: [2.0_f64.powi(1000), 0.0, 0.0],
        ..Fixture::BASIC
    };
    let (scene, _) = import(fixture).into_parts();
    let target = scene
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap()
        .id();
    let generation = scene.generation();
    let failure = generate_tangents(
        scene,
        target,
        TangentOptions::new(AttributeDomain::Corner, 0, AttributeDomain::Corner),
        PostProcessControl::default(),
    )
    .unwrap_err();
    assert_eq!(failure.input().generation(), generation);
    assert!(
        matches!(failure.error(), PostProcessRunError::Tangents { failure }
        if matches!(failure.cause(), TangentCause::Issue { issue: TangentIssue::Scaling {
            semantic: baozi_core::AttributeSemanticKind::Position,
            element: Some(1), issue: GeometryNumericIssue::ScalingUnderflow,
        }}))
    );
}
