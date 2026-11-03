use baozi_core::{
    AttributeData, AttributeDomain, AttributeElement, FormatId, PrimitiveRef, SceneSpace,
    TopologyKind,
};
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
    PostProcessControl, PostProcessRunDisposition, PostProcessRunError, PostProcessRunOutcome,
    TriangulationCause, TriangulationIssue, triangulate_primitive,
};

const FORMAT: &str = "dev.baozi.test.triangulation-precision";
const CORNER_VALUES: [f64; 4] = [
    -0.0,
    f64::from_bits(1),
    f64::from_bits(0x3ff0_0000_0000_0001),
    1.0e250,
];

#[derive(Clone, Copy, Debug)]
enum Storage {
    Dense,
    Sparse { logical_count: u32 },
}

#[derive(Clone, Copy)]
struct Fixture {
    positions: [[f64; 3]; 4],
    storage: Storage,
}

impl Fixture {
    fn vertices(self) -> [u32; 4] {
        match self.storage {
            Storage::Dense => [0, 1, 2, 3],
            Storage::Sparse { logical_count } => [0, 1, 2, logical_count - 2],
        }
    }
}

struct PrecisionImporter(Fixture);

impl Format for PrecisionImporter {
    type Options = NoFormatOptions;

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> Result<CompiledOptions<Self::Options>, OptionCompileError> {
        NoFormatOptions::compile_defaults(scope)
    }

    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new(FORMAT).unwrap(),
            "Triangulation precision fixture",
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

    fn parse(&self, _options: &Self::Options, context: &mut ParseContext<'_>) -> ParseResult<()> {
        let explicit_indices = match self.0.storage {
            Storage::Dense => [0, 1, 2, 3],
            Storage::Sparse { logical_count } => [0, 1, 2, logical_count - 1],
        };
        let position = match self.0.storage {
            Storage::Dense => GeometryAttributeAuthoring::f64(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                self.0.positions.as_flattened(),
            ),
            Storage::Sparse { logical_count } => GeometryAttributeAuthoring::sparse_f64(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                logical_count,
                &explicit_indices,
                self.0.positions.as_flattened(),
                &self.0.positions[3],
            ),
        };
        let attributes = [
            position,
            GeometryAttributeAuthoring::sparse_f64(
                AttributeSemantic::Texcoord(0),
                AttributeDomain::Corner,
                2,
                4,
                &[0, 2],
                &CORNER_VALUES,
                &CORNER_VALUES[..2],
            ),
        ];
        let indices = self.0.vertices();
        let primitives = [GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::Polygons(&[4]),
            Presence::Present(&indices),
            baozi_core::MaterialAssignment::Unspecified,
            AuthoringRange::new(0, 2),
        )];
        let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
        context.submit(
            StaticSceneBuilder::new(Presence::Present(&[]), &[])
                .with_scene_space(&SceneSpace::STANDARD_TARGET)
                .with_geometry(&meshes, &primitives, &attributes, &[]),
        )
    }
}

fn import_fixture(fixture: Fixture) -> ImportReport {
    let mut registry = RegistryBuilder::new();
    registry.register(PrecisionImporter(fixture)).unwrap();
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
        .expect("finite F64 geometry must pass public authoring and import validation")
}

fn assert_bits(actual: &[f64], expected: &[f64]) {
    assert_eq!(
        actual.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    );
}

fn assert_positions(primitive: PrimitiveRef<'_>, fixture: Fixture) {
    let position = primitive.positions().unwrap();
    match (fixture.storage, position.data()) {
        (Storage::Dense, AttributeData::DenseF64 { lanes: 3, values }) => {
            assert_eq!(primitive.vertex_count(), 4);
            assert_bits(values, fixture.positions.as_flattened());
        }
        (
            Storage::Sparse { logical_count },
            AttributeData::SparseF64 {
                lanes: 3,
                logical_count: actual_count,
                indices,
                values,
                default,
            },
        ) => {
            assert_eq!(primitive.vertex_count(), logical_count);
            assert_eq!(actual_count, logical_count);
            assert_eq!(indices, &[0, 1, 2, logical_count - 1]);
            assert_bits(values, fixture.positions.as_flattened());
            assert_bits(default, &fixture.positions[3]);
        }
        (_, data) => panic!(
            "Position encoding must remain {:?}, got {data:?}",
            fixture.storage
        ),
    }
    for (vertex, expected) in fixture.vertices().into_iter().zip(&fixture.positions) {
        let Some(AttributeElement::F64(actual)) = position.element(vertex as usize) else {
            panic!("logical Position reads must retain F64");
        };
        assert_bits(actual, expected);
    }
}

fn triangulate(fixture: Fixture) -> PostProcessRunOutcome {
    let (scene, import_evidence) = import_fixture(fixture).into_parts();
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    assert_positions(primitive, fixture);
    let target = primitive.id();
    let outcome = triangulate_primitive(scene, target, PostProcessControl::default())
        .expect("a well-conditioned F64 quad must triangulate without narrowing");
    let report = outcome.report();
    assert_eq!(report.disposition(), PostProcessRunDisposition::Changed);
    assert!(report.matches_scene(outcome.scene()));
    assert!(!import_evidence.matches_scene(outcome.scene()));
    assert_eq!(report.resources().current_live_bytes(), 0);
    assert_eq!(report.resources().active_reservations(), 0);
    let remap = report.triangulation_remap().unwrap();
    assert_eq!(remap.input_target(), target);
    assert_eq!(remap.source_face_triangle_offsets(), &[0, 2]);
    let primitive = outcome.scene().primitive(remap.output_target()).unwrap();
    assert_eq!(primitive.topology(), TopologyKind::TriangleList);
    assert_eq!(primitive.face_count(), 2);
    assert_eq!(primitive.corner_count(), 6);
    assert_positions(primitive, fixture);
    let origins = remap.source_corner_origins();
    assert_eq!(origins.len(), 6);
    let vertices = fixture.vertices();
    assert_eq!(
        primitive.indices().unwrap(),
        origins
            .iter()
            .map(|&origin| vertices[origin as usize])
            .collect::<Vec<_>>()
    );
    let AttributeData::SparseF64 {
        lanes: 2,
        logical_count: 6,
        indices,
        values,
        default,
    } = primitive
        .texcoords(0, AttributeDomain::Corner)
        .unwrap()
        .data()
    else {
        panic!("corner remapping must retain sparse F64, explicit values and default");
    };
    let explicit = origins
        .iter()
        .enumerate()
        .filter(|&(_, &origin)| origin == 0 || origin == 2)
        .collect::<Vec<_>>();
    assert_eq!(
        indices,
        explicit
            .iter()
            .map(|&(index, _)| index as u32)
            .collect::<Vec<_>>()
    );
    let expected_values = explicit
        .iter()
        .flat_map(|&(_, &origin)| {
            if origin == 0 {
                &CORNER_VALUES[..2]
            } else {
                &CORNER_VALUES[2..]
            }
        })
        .copied()
        .collect::<Vec<_>>();
    assert_bits(values, &expected_values);
    assert_bits(default, &CORNER_VALUES[..2]);
    outcome
}

fn adjacent_large_positions() -> [[f64; 3]; 4] {
    let low = 1.0e15_f64;
    let high = f64::from_bits(low.to_bits() + 1);
    [
        [low, low, -0.0],
        [high, low, 0.0],
        [high, high, -0.0],
        [low, high, 0.0],
    ]
}

#[test]
fn precision_contract_dense_f64_adjacent_large_positions_triangulate_without_narrowing() {
    let _ = triangulate(Fixture {
        positions: adjacent_large_positions(),
        storage: Storage::Dense,
    });
}

#[test]
fn precision_contract_sparse_f64_adjacent_large_positions_triangulate_without_narrowing() {
    let _ = triangulate(Fixture {
        positions: adjacent_large_positions(),
        storage: Storage::Sparse { logical_count: 5 },
    });
}

#[test]
fn precision_contract_f64_tiny_polygons_retain_position_and_corner_bits() {
    for scale in [f64::from_bits(1), 1.0e-200] {
        for storage in [Storage::Dense, Storage::Sparse { logical_count: 5 }] {
            let _ = triangulate(Fixture {
                positions: [
                    [-scale, -scale, -0.0],
                    [scale, -scale, 0.0],
                    [scale, scale, -0.0],
                    [-scale, scale, 0.0],
                ],
                storage,
            });
        }
    }
}

#[test]
fn precision_contract_sparse_f64_large_logical_domain_keeps_work_and_storage_bounded() {
    let outcome = triangulate(Fixture {
        positions: adjacent_large_positions(),
        storage: Storage::Sparse {
            logical_count: 1_000_003,
        },
    });
    let resources = outcome.report().resources();
    assert!(resources.peak_staging_bytes() < 64 * 1024);
    assert!(resources.committed_canonical_bytes() < 64 * 1024);
    assert!(resources.attempted_work_actions() < 100_000);
}

#[test]
fn precision_contract_degenerate_f64_polygon_returns_numeric_cause_and_original_scene() {
    for storage in [Storage::Dense, Storage::Sparse { logical_count: 5 }] {
        let fixture = Fixture {
            positions: [
                [0.0, -0.0, 0.0],
                [1.0, 0.0, -0.0],
                [2.0, -0.0, 0.0],
                [3.0, 0.0, -0.0],
            ],
            storage,
        };
        let (scene, import_evidence) = import_fixture(fixture).into_parts();
        let target = scene
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap()
            .id();
        let failure = triangulate_primitive(scene, target, PostProcessControl::default())
            .expect_err("F64 collinear vertices must reject without publishing a new generation");
        assert!(import_evidence.matches_scene(failure.input()));
        assert_eq!(failure.input().generation(), target.generation());
        let primitive = failure.input().primitive(target).unwrap();
        assert_eq!(primitive.topology(), TopologyKind::Polygons);
        assert_positions(primitive, fixture);
        let PostProcessRunError::Triangulation { failure } = failure.error() else {
            panic!("F64 numeric rejection must retain triangulation evidence");
        };
        assert!(matches!(
            failure.cause(),
            TriangulationCause::Polygon {
                face: 0,
                issue: TriangulationIssue::Degenerate
            }
        ));
        let resources = failure.resources().unwrap();
        assert_eq!(resources.current_live_bytes(), 0);
        assert_eq!(resources.active_reservations(), 0);
        assert_eq!(resources.committed_canonical_bytes(), 0);
    }
}
