use baozi_core::{
    AttributeData, AttributeDomain, AttributeElement, FormatId, PrimitiveRef, SceneSpace,
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
    ExistingNormals, GeometryNumericIssue, NormalCause, NormalIssue, NormalMode, NormalOptions,
    PostProcessControl, PostProcessRunDisposition, PostProcessRunError, PostProcessRunOutcome,
    generate_normals,
};

const FORMAT: &str = "dev.baozi.test.normals-precision";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Width {
    F32,
    F64,
}

#[derive(Clone, Copy, Debug)]
enum Storage {
    Dense,
    Sparse { logical_count: u32 },
}

#[derive(Clone)]
struct Fixture {
    positions: Vec<[f64; 3]>,
    width: Width,
    storage: Storage,
    indices: Vec<u32>,
    normal: Option<Width>,
    groups: Option<Vec<u32>>,
}

impl Fixture {
    fn triangle(positions: [[f64; 3]; 3], storage: Storage) -> Self {
        Self {
            positions: positions.to_vec(),
            width: Width::F64,
            storage,
            indices: vec![0, 1, 2],
            normal: None,
            groups: None,
        }
    }

    fn vertex_count(&self) -> u32 {
        match self.storage {
            Storage::Dense => self.positions.len() as u32,
            Storage::Sparse { logical_count } => logical_count,
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
        FormatDescriptor::builder(FormatId::new(FORMAT).unwrap(), "Normal precision fixture")
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
        let fixture = &self.0;
        let narrow: Vec<f32> = match fixture.width {
            Width::F32 => fixture
                .positions
                .as_flattened()
                .iter()
                .map(|&v| v as f32)
                .collect(),
            Width::F64 => Vec::new(),
        };
        let explicit: Vec<u32> = (1..fixture.positions.len() as u32).collect();
        // Vertex zero and all unused sparse vertices read the authored default.
        let position = match (fixture.width, fixture.storage) {
            (Width::F32, Storage::Dense) => GeometryAttributeAuthoring::f32(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                &narrow,
                Presence::Absent,
            ),
            (Width::F64, Storage::Dense) => GeometryAttributeAuthoring::f64(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                fixture.positions.as_flattened(),
            ),
            (Width::F32, Storage::Sparse { logical_count }) => {
                GeometryAttributeAuthoring::sparse_f32(
                    AttributeSemantic::Position,
                    AttributeDomain::Vertex,
                    3,
                    logical_count,
                    &explicit,
                    &narrow[3..],
                    &narrow[..3],
                )
            }
            (Width::F64, Storage::Sparse { logical_count }) => {
                GeometryAttributeAuthoring::sparse_f64(
                    AttributeSemantic::Position,
                    AttributeDomain::Vertex,
                    3,
                    logical_count,
                    &explicit,
                    fixture.positions[1..].as_flattened(),
                    &fixture.positions[0],
                )
            }
        };
        let mut attributes = vec![position];
        if let Some(width) = fixture.normal {
            attributes.push(match width {
                Width::F32 => GeometryAttributeAuthoring::sparse_f32(
                    AttributeSemantic::Normal,
                    AttributeDomain::Vertex,
                    3,
                    fixture.vertex_count(),
                    &[],
                    &[],
                    &[0.0, 0.0, 1.0],
                ),
                Width::F64 => GeometryAttributeAuthoring::sparse_f64(
                    AttributeSemantic::Normal,
                    AttributeDomain::Vertex,
                    3,
                    fixture.vertex_count(),
                    &[],
                    &[],
                    &[0.0, 0.0, 1.0],
                ),
            });
        }
        if let Some(groups) = &fixture.groups {
            attributes.push(GeometryAttributeAuthoring::u32(
                AttributeSemantic::SmoothingGroup,
                AttributeDomain::Face,
                1,
                groups,
            ));
        }
        let primitives = [GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            if fixture.indices.is_empty() {
                Presence::Absent
            } else {
                Presence::Present(&fixture.indices)
            },
            baozi_core::MaterialAssignment::Unspecified,
            AuthoringRange::new(0, attributes.len() as u32),
        )];
        let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
        context.submit(
            StaticSceneBuilder::new(Presence::Present(&[]), &[])
                .with_scene_space(&SceneSpace::STANDARD_TARGET)
                .with_geometry(&meshes, &primitives, &attributes, &[]),
        )
    }
}

fn import_fixture(fixture: &Fixture) -> ImportReport {
    let mut registry = RegistryBuilder::new();
    registry
        .register(PrecisionImporter(fixture.clone()))
        .unwrap();
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
            AssetPath::new("normals.fixture").unwrap(),
            b"precision",
            ImportOperationControl::default(),
        )
        .expect("finite positions must pass public authoring and import validation")
}

fn assert_f64_bits(actual: &[f64], expected: &[f64]) {
    assert_eq!(
        actual.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
    );
}

fn assert_f32_bits(actual: &[f32], expected: &[f64]) {
    assert_eq!(
        actual.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        expected
            .iter()
            .map(|&v| (v as f32).to_bits())
            .collect::<Vec<_>>()
    );
}

fn assert_positions(primitive: PrimitiveRef<'_>, fixture: &Fixture) {
    let position = primitive.positions().unwrap();
    assert_eq!(primitive.vertex_count(), fixture.vertex_count());
    match (fixture.width, fixture.storage, position.data()) {
        (Width::F32, Storage::Dense, AttributeData::DenseF32 { lanes: 3, values }) => {
            assert_f32_bits(values, fixture.positions.as_flattened());
        }
        (Width::F64, Storage::Dense, AttributeData::DenseF64 { lanes: 3, values }) => {
            assert_f64_bits(values, fixture.positions.as_flattened());
        }
        (
            Width::F32,
            Storage::Sparse { logical_count },
            AttributeData::SparseF32 {
                lanes: 3,
                logical_count: actual,
                indices,
                values,
                default,
            },
        ) => {
            assert_eq!(actual, logical_count);
            assert_eq!(
                indices,
                (1..fixture.positions.len() as u32).collect::<Vec<_>>()
            );
            assert_f32_bits(values, fixture.positions[1..].as_flattened());
            assert_f32_bits(default, &fixture.positions[0]);
        }
        (
            Width::F64,
            Storage::Sparse { logical_count },
            AttributeData::SparseF64 {
                lanes: 3,
                logical_count: actual,
                indices,
                values,
                default,
            },
        ) => {
            assert_eq!(actual, logical_count);
            assert_eq!(
                indices,
                (1..fixture.positions.len() as u32).collect::<Vec<_>>()
            );
            assert_f64_bits(values, fixture.positions[1..].as_flattened());
            assert_f64_bits(default, &fixture.positions[0]);
        }
        (_, _, data) => panic!("Position width and encoding must be retained, got {data:?}"),
    }
    for (vertex, expected) in fixture.positions.iter().enumerate() {
        match (fixture.width, position.element(vertex).unwrap()) {
            (Width::F32, AttributeElement::F32(value)) => assert_f32_bits(value, expected),
            (Width::F64, AttributeElement::F64(value)) => assert_f64_bits(value, expected),
            (_, value) => panic!("logical Position reads changed width: {value:?}"),
        }
    }
    if fixture.indices.is_empty() {
        assert!(primitive.indices().is_none());
    } else {
        assert_eq!(primitive.indices().unwrap(), fixture.indices);
    }
}

fn generate(fixture: &Fixture, mode: NormalMode) -> PostProcessRunOutcome {
    let (scene, import_evidence) = import_fixture(fixture).into_parts();
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    assert_positions(primitive, fixture);
    let target = primitive.id();
    let result = generate_normals(
        scene,
        target,
        NormalOptions {
            mode,
            existing: ExistingNormals::Replace,
        },
        baozi_postprocess::TangentPolicy::Preserve,
        PostProcessControl::default(),
    )
    .expect("well-conditioned normals must publish at retained width");
    let report = result.report();
    assert_eq!(report.disposition(), PostProcessRunDisposition::Changed);
    assert!(report.matches_scene(result.scene()));
    assert!(!import_evidence.matches_scene(result.scene()));
    assert_eq!(report.resources().current_live_bytes(), 0);
    assert_eq!(report.resources().active_reservations(), 0);
    let generation = report.normal_generation().unwrap();
    assert_eq!(
        generation.replaced_authored_normals(),
        fixture.normal.is_some()
    );
    assert_positions(
        result
            .scene()
            .primitive(generation.output_target())
            .unwrap(),
        fixture,
    );
    result
}

fn assert_normals(result: &PostProcessRunOutcome, width: Width, expected: &[[f64; 3]]) {
    let primitive = result
        .scene()
        .primitive(result.report().normal_generation().unwrap().output_target())
        .unwrap();
    let data = primitive.normals(AttributeDomain::Corner).unwrap().data();
    let (values, tolerance): (Vec<[f64; 3]>, f64) = match (width, data) {
        (Width::F32, AttributeData::DenseF32 { lanes: 3, values }) => {
            for &value in values {
                if value == 0.0 {
                    assert_eq!(value.to_bits(), 0);
                }
            }
            (
                values
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|v| v.map(f64::from))
                    .collect(),
                2.0e-6,
            )
        }
        (Width::F64, AttributeData::DenseF64 { lanes: 3, values }) => {
            for &value in values {
                if value == 0.0 {
                    assert_eq!(value.to_bits(), 0);
                }
            }
            (values.as_chunks::<3>().0.to_vec(), 8.0 * f64::EPSILON)
        }
        (_, data) => panic!("generated Normal must have selected width {width:?}, got {data:?}"),
    };
    assert_eq!(values.len(), expected.len());
    for (actual, expected) in values.iter().zip(expected) {
        for axis in 0..3 {
            assert!(actual[axis].is_finite());
            assert!(
                (actual[axis] - expected[axis]).abs() <= tolerance,
                "{actual:?} != {expected:?}"
            );
        }
    }
}

fn reject(fixture: &Fixture, mode: NormalMode) -> NormalIssue {
    let (scene, evidence) = import_fixture(fixture).into_parts();
    let target = scene
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap()
        .id();
    let failure = generate_normals(
        scene,
        target,
        NormalOptions {
            mode,
            ..NormalOptions::default()
        },
        baozi_postprocess::TangentPolicy::Preserve,
        PostProcessControl::default(),
    )
    .expect_err("unresolved geometry must not publish a generation");
    assert!(evidence.matches_scene(failure.input()));
    assert_eq!(failure.input().generation(), target.generation());
    let primitive = failure.input().primitive(target).unwrap();
    assert_positions(primitive, fixture);
    assert!(primitive.normals(AttributeDomain::Corner).is_none());
    let PostProcessRunError::Normals { failure } = failure.error() else {
        panic!("typed normal failure");
    };
    let resources = failure.resources().unwrap();
    assert_eq!(resources.current_live_bytes(), 0);
    assert_eq!(resources.active_reservations(), 0);
    assert_eq!(resources.committed_canonical_bytes(), 0);
    let NormalCause::Issue { issue } = failure.cause() else {
        panic!("numeric issue: {failure:?}");
    };
    issue.clone()
}

fn power_of_two(exponent: i32) -> f64 {
    assert!((-1022..=1023).contains(&exponent));
    f64::from_bits(((exponent + 1023) as u64) << 52)
}

#[test]
fn dense_and_sparse_f64_adjacent_huge_positions_generate_normals_and_retain_bits() {
    let base = power_of_two(1000);
    let adjacent = f64::from_bits(base.to_bits() + 1);
    for storage in [Storage::Dense, Storage::Sparse { logical_count: 7 }] {
        for (indices, sign) in [
            ([0, 1, 2], 1.0),
            ([1, 2, 0], 1.0),
            ([2, 0, 1], 1.0),
            ([0, 2, 1], -1.0),
            ([2, 1, 0], -1.0),
            ([1, 0, 2], -1.0),
        ] {
            let mut fixture = Fixture::triangle(
                [
                    [base, base, -0.0],
                    [adjacent, base, 0.0],
                    [base, adjacent, -0.0],
                ],
                storage,
            );
            fixture.indices = indices.to_vec();
            for mode in [NormalMode::Flat, NormalMode::Smooth] {
                assert_normals(
                    &generate(&fixture, mode),
                    Width::F64,
                    &[[0.0, 0.0, sign]; 3],
                );
            }
        }
    }
}

#[test]
fn f64_tiny_subnormal_and_opposite_extreme_faces_generate_without_raw_area_overflow() {
    let mut cases: Vec<[[f64; 3]; 3]> =
        [power_of_two(-1000), f64::from_bits(1), power_of_two(1000)]
            .into_iter()
            .map(|s| [[-0.0, 0.0, -0.0], [s, 0.0, 0.0], [0.0, s, -0.0]])
            .collect();
    cases.push([
        [-f64::MAX, 0.0, -0.0],
        [f64::MAX, 0.0, 0.0],
        [0.0, f64::MAX, -0.0],
    ]);
    for positions in cases {
        for storage in [Storage::Dense, Storage::Sparse { logical_count: 7 }] {
            for mode in [NormalMode::Flat, NormalMode::Smooth] {
                assert_normals(
                    &generate(&Fixture::triangle(positions, storage), mode),
                    Width::F64,
                    &[[0.0, 0.0, 1.0]; 3],
                );
            }
        }
    }
}

#[test]
fn f64_smooth_normals_use_one_primitive_scale_and_analytic_area_weights() {
    let shared = [0.0, 1.0 / 17.0_f64.sqrt(), 4.0 / 17.0_f64.sqrt()];
    for exponent in [-900, 0, 900] {
        let scale = power_of_two(exponent);
        for storage in [
            Storage::Dense,
            Storage::Sparse { logical_count: 6 },
            Storage::Sparse { logical_count: 9 },
        ] {
            let fixture = Fixture {
                positions: vec![
                    [-0.0, 0.0, -0.0],
                    [0.5 * scale, 0.0, 0.0],
                    [0.0, 0.5 * scale, 0.0],
                    [0.0, 0.0, 0.25 * scale],
                    [0.25 * scale, 0.0, 0.0],
                ],
                width: Width::F64,
                storage,
                indices: vec![0, 1, 2, 0, 3, 4],
                normal: None,
                groups: None,
            };
            assert_normals(
                &generate(&fixture, NormalMode::Smooth),
                Width::F64,
                &[
                    shared,
                    [0.0, 0.0, 1.0],
                    [0.0, 0.0, 1.0],
                    shared,
                    [0.0, 1.0, 0.0],
                    [0.0, 1.0, 0.0],
                ],
            );
        }
    }
}

#[test]
fn f64_smooth_cancellation_is_a_bucket_uncertainty_with_original_scene() {
    for storage in [
        Storage::Dense,
        Storage::Sparse { logical_count: 6 },
        Storage::Sparse { logical_count: 7 },
    ] {
        let mut fixture = Fixture::triangle([[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], storage);
        fixture.indices = vec![1, 2, 0, 0, 2, 1];
        for groups in [None, Some(vec![7, 7])] {
            fixture.groups = groups;
            assert_eq!(
                reject(&fixture, NormalMode::Smooth),
                NormalIssue::SmoothNumerical {
                    vertex: 0,
                    group: fixture.groups.as_ref().map(|g| g[0]),
                    corner: 2,
                    issue: GeometryNumericIssue::UncertainDirection,
                }
            );
        }
        fixture.groups = Some(vec![0, 0]);
        assert_normals(
            &generate(&fixture, NormalMode::Smooth),
            Width::F64,
            &[
                [0.0, 0.0, 1.0],
                [0.0, 0.0, 1.0],
                [0.0, 0.0, 1.0],
                [0.0, 0.0, -1.0],
                [0.0, 0.0, -1.0],
                [0.0, 0.0, -1.0],
            ],
        );
    }
}

#[test]
fn sparse_f32_bucket_area_weights_retain_f64_replacement_width() {
    let shared = [0.0, 1.0 / 17.0_f64.sqrt(), 4.0 / 17.0_f64.sqrt()];
    for storage in [Storage::Dense, Storage::Sparse { logical_count: 6 }] {
        for groups in [None, Some(vec![7, 7])] {
            let fixture = Fixture {
                positions: vec![
                    [-0.0, 0.0, -0.0],
                    [0.5, 0.0, 0.0],
                    [0.0, 0.5, 0.0],
                    [0.0, 0.0, 0.25],
                    [0.25, 0.0, 0.0],
                ],
                width: Width::F32,
                storage,
                indices: vec![0, 1, 2, 0, 3, 4],
                normal: Some(Width::F64),
                groups,
            };
            assert_normals(
                &generate(&fixture, NormalMode::Smooth),
                Width::F64,
                &[
                    shared,
                    [0.0, 0.0, 1.0],
                    [0.0, 0.0, 1.0],
                    shared,
                    [0.0, 1.0, 0.0],
                    [0.0, 1.0, 0.0],
                ],
            );
        }
    }
}

#[test]
fn f64_uncertain_face_and_witnessed_degeneracy_have_distinct_numeric_causes() {
    for storage in [Storage::Dense, Storage::Sparse { logical_count: 7 }] {
        for mode in [NormalMode::Flat, NormalMode::Smooth] {
            let t = power_of_two(-54);
            let d = power_of_two(-51);
            assert_eq!(
                reject(
                    &Fixture::triangle(
                        [[t, t, 0.0], [1.0, 1.0, 0.0], [2.0, 2.0 + d, 0.0]],
                        storage
                    ),
                    mode
                ),
                NormalIssue::Numerical {
                    face: 0,
                    issue: GeometryNumericIssue::UncertainDirection
                }
            );
            assert_eq!(
                reject(
                    &Fixture::triangle([[0.0; 3], [1.0; 3], [2.0; 3]], storage),
                    mode
                ),
                NormalIssue::Numerical {
                    face: 0,
                    issue: GeometryNumericIssue::ExactDegenerateFace
                }
            );
        }
    }
}

#[test]
fn f64_scaling_loss_reports_the_original_position_component() {
    for (maximum, tiny, issue) in [
        (
            2.0,
            f64::from_bits(0x0010_0000_0000_0001),
            GeometryNumericIssue::ScalingPrecisionLoss,
        ),
        (
            power_of_two(1000),
            f64::from_bits(1),
            GeometryNumericIssue::ScalingUnderflow,
        ),
    ] {
        for storage in [Storage::Dense, Storage::Sparse { logical_count: 7 }] {
            for mode in [NormalMode::Flat, NormalMode::Smooth] {
                assert_eq!(
                    reject(
                        &Fixture::triangle(
                            [[0.0; 3], [maximum, 0.0, 0.0], [0.0, tiny, 0.0]],
                            storage
                        ),
                        mode
                    ),
                    NormalIssue::PositionNumerical {
                        vertex: 2,
                        component: 1,
                        issue
                    }
                );
            }
        }
    }
}

#[test]
fn f32_positions_replacing_f64_normals_publish_actual_dense_f64_values() {
    let epsilon = f64::from(f32::from_bits(1));
    let expected = [
        1.0 / 6.0_f64.sqrt(),
        -1.0 / 6.0_f64.sqrt(),
        -2.0 / 6.0_f64.sqrt(),
    ];
    for storage in [Storage::Dense, Storage::Sparse { logical_count: 7 }] {
        let mut fixture = Fixture::triangle(
            [
                [1.0, 1.0, -0.0],
                [epsilon, 0.0, epsilon],
                [0.0, epsilon, 0.0],
            ],
            storage,
        );
        fixture.width = Width::F32;
        fixture.normal = Some(Width::F64);
        for mode in [NormalMode::Flat, NormalMode::Smooth] {
            // The F64 specialization would reject these poorly conditioned edge differences.
            // Declared F32 operands retain the exact-product specialization even for F64 output.
            assert_normals(&generate(&fixture, mode), Width::F64, &[expected; 3]);
        }
    }
}

#[test]
fn f32_only_inputs_keep_f32_output_and_f64_position_widens_f32_replacement() {
    let positions = [[-0.0, 0.0, -0.0], [1.0, 0.0, 2.0], [0.0, 1.0, -3.0]];
    let expected = [
        -2.0 / 14.0_f64.sqrt(),
        3.0 / 14.0_f64.sqrt(),
        1.0 / 14.0_f64.sqrt(),
    ];
    for storage in [Storage::Dense, Storage::Sparse { logical_count: 7 }] {
        for mode in [NormalMode::Flat, NormalMode::Smooth] {
            for normal in [None, Some(Width::F32)] {
                let mut fixture = Fixture::triangle(positions, storage);
                fixture.width = Width::F32;
                fixture.normal = normal;
                assert_normals(&generate(&fixture, mode), Width::F32, &[expected; 3]);
                fixture.width = Width::F64;
                assert_normals(&generate(&fixture, mode), Width::F64, &[expected; 3]);
            }
        }
    }
}

#[test]
fn sparse_f64_logical_domain_is_scanned_without_densifying_positions() {
    let logical_count = 100_003;
    let fixture = Fixture::triangle(
        [[-0.0, 0.0, -0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        Storage::Sparse { logical_count },
    );
    let result = generate(&fixture, NormalMode::Smooth);
    assert_normals(&result, Width::F64, &[[0.0, 0.0, 1.0]; 3]);
    let resources = result.report().resources();
    assert!(resources.peak_staging_bytes() < 64 * 1024);
    assert!(resources.committed_canonical_bytes() < 64 * 1024);
    assert!(resources.attempted_work_actions() > u64::from(logical_count));
    assert!(resources.attempted_work_actions() < 8 * u64::from(logical_count) + 10_000);
}

#[test]
fn nonindexed_f64_faces_do_not_weld_equal_positions() {
    for storage in [Storage::Dense, Storage::Sparse { logical_count: 6 }] {
        let fixture = Fixture {
            positions: vec![
                [-0.0, 0.0, -0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [-0.0, 0.0, -0.0],
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 0.0],
            ],
            width: Width::F64,
            storage,
            indices: vec![],
            normal: None,
            groups: None,
        };
        for mode in [NormalMode::Flat, NormalMode::Smooth] {
            assert_normals(
                &generate(&fixture, mode),
                Width::F64,
                &[
                    [0.0, 0.0, 1.0],
                    [0.0, 0.0, 1.0],
                    [0.0, 0.0, 1.0],
                    [0.0, 1.0, 0.0],
                    [0.0, 1.0, 0.0],
                    [0.0, 1.0, 0.0],
                ],
            );
        }
    }
}

#[test]
fn preserving_authored_f64_normals_keeps_the_original_generation_without_kernel_work() {
    let mut fixture = Fixture::triangle(
        [[0.0; 3], [1.0; 3], [2.0; 3]],
        Storage::Sparse { logical_count: 7 },
    );
    fixture.normal = Some(Width::F64);
    let (scene, evidence) = import_fixture(&fixture).into_parts();
    let target = scene
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap()
        .id();
    let result = generate_normals(
        scene,
        target,
        NormalOptions::default(),
        baozi_postprocess::TangentPolicy::Preserve,
        PostProcessControl::default(),
    )
    .unwrap();
    assert!(evidence.matches_scene(result.scene()));
    assert_eq!(
        result.report().disposition(),
        PostProcessRunDisposition::Unchanged
    );
    assert!(result.report().normal_generation().is_none());
    let primitive = result.scene().primitive(target).unwrap();
    assert_positions(primitive, &fixture);
    let AttributeData::SparseF64 {
        indices,
        values,
        default,
        ..
    } = primitive.normals(AttributeDomain::Vertex).unwrap().data()
    else {
        panic!("preserve retains authored sparse F64 Normal");
    };
    assert!(indices.is_empty());
    assert!(values.is_empty());
    assert_f64_bits(default, &[0.0, 0.0, 1.0]);
}
