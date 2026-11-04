use std::{path::PathBuf, sync::Arc};

use baozi::{
    AssetPath, AttributeElement, AttributeSemanticRef, Importer, MemoryAssetIo, MorphDeltaSemantic,
    TargetSpace, ValidatedScene,
    postprocess::{
        ListVertex, PostProcessControl, SpaceConversionOptions, convert_scene_space,
        triangle_vertices,
    },
};
use baozi_asset_consumer::{ApplicationAsset, DrawBuffer, Options, Result, process, process_file};
use baozi_native_io_candidate::{FileSystemAssetIo, NativeDirectoryCapability};
use serde_json::Value;

#[derive(Debug, PartialEq, Eq)]
struct CornerBits {
    position: [u64; 3],
    normal: Option<[u64; 3]>,
    tangent: Option<[u64; 4]>,
    color: Option<[u64; 4]>,
    uvs: Vec<(u32, [u64; 2])>,
    influences: Option<(Vec<u32>, Vec<u32>)>,
    morphs: Vec<(usize, MorphDeltaSemantic, [u32; 3])>,
}

fn field_bits<const N: usize>(element: AttributeElement<'_>) -> [u64; N] {
    match element {
        AttributeElement::F32(values) => {
            assert_eq!(values.len(), N);
            std::array::from_fn(|i| f64::from(values[i]).to_bits())
        }
        AttributeElement::F64(values) => {
            assert_eq!(values.len(), N);
            std::array::from_fn(|i| values[i].to_bits())
        }
        _ => panic!("expected a floating application field"),
    }
}

fn selected_bits<const N: usize>(
    tuple: ListVertex<'_>,
    semantic: AttributeSemanticRef<'_>,
) -> Option<[u64; N]> {
    let mut fields = tuple
        .attributes()
        .filter(|field| field.attribute().semantic() == semantic);
    let first = fields.next();
    assert!(
        fields.next().is_none(),
        "fixture has ambiguous selected domains"
    );
    first.map(|field| field_bits(field.element()))
}

fn source_corner(tuple: ListVertex<'_>) -> Result<CornerBits> {
    let mut uvs = Vec::new();
    for field in tuple.attributes() {
        if let AttributeSemanticRef::Texcoord(set) = field.attribute().semantic() {
            uvs.push((set, field_bits(field.element())));
        }
    }
    uvs.sort_unstable_by_key(|&(set, _)| set);
    let mut morphs = Vec::new();
    for (target_index, target) in tuple.primitive().morph_targets().enumerate() {
        for delta in target.deltas() {
            let value = tuple
                .morph_delta(target.id(), delta.semantic())?
                .ok_or("missing source morph stream")?;
            morphs.push((
                target_index,
                delta.semantic(),
                [value.x, value.y, value.z].map(f32::to_bits),
            ));
        }
    }
    let mut colors = tuple
        .attributes()
        .filter(|field| field.attribute().semantic() == AttributeSemanticRef::Color(0));
    let color = colors.next().map(|field| {
        let mut rgba = [0.0, 0.0, 0.0, 1.0];
        match field.element() {
            AttributeElement::F32(values) => {
                assert!(matches!(values.len(), 3 | 4));
                for (output, value) in rgba.iter_mut().zip(values) {
                    *output = f64::from(*value);
                }
            }
            AttributeElement::F64(values) => {
                assert!(matches!(values.len(), 3 | 4));
                rgba[..values.len()].copy_from_slice(values);
            }
            _ => panic!("expected a floating color field"),
        }
        rgba.map(f64::to_bits)
    });
    assert!(
        colors.next().is_none(),
        "fixture has ambiguous color domains"
    );
    Ok(CornerBits {
        position: selected_bits(tuple, AttributeSemanticRef::Position)
            .ok_or("missing source positions")?,
        normal: selected_bits(tuple, AttributeSemanticRef::Normal),
        tangent: selected_bits(tuple, AttributeSemanticRef::Tangent),
        color,
        uvs,
        influences: tuple.influences().map(|row| {
            (
                row.joint_indices().to_vec(),
                row.weights().iter().map(|v| v.to_bits()).collect(),
            )
        }),
        morphs,
    })
}

fn expanded_corners(scene: &ValidatedScene) -> Result<Vec<Vec<CornerBits>>> {
    let mut draws = Vec::new();
    for mesh in scene.meshes() {
        for primitive in mesh.primitives() {
            draws.push(
                triangle_vertices(primitive)?
                    .vertices()
                    .map(source_corner)
                    .collect::<Result<Vec<_>>>()?,
            );
        }
    }
    Ok(draws)
}

fn owned_corner(draw: &DrawBuffer, index: usize) -> CornerBits {
    let vertex = &draw.vertices[index];
    CornerBits {
        position: vertex.position.map(f64::to_bits),
        normal: vertex.normal.map(|v| v.map(f64::to_bits)),
        tangent: vertex.tangent.map(|v| v.map(f64::to_bits)),
        color: vertex.color.map(|v| v.map(f64::to_bits)),
        uvs: draw
            .uv_streams
            .iter()
            .map(|stream| (stream.set, stream.values[index].map(f64::to_bits)))
            .collect(),
        influences: draw.influences.as_ref().map(|rows| {
            let range = rows.offsets[index]..rows.offsets[index + 1];
            (
                rows.joints[range.clone()].to_vec(),
                rows.weights[range].iter().map(|v| v.to_bits()).collect(),
            )
        }),
        morphs: draw
            .morphs
            .iter()
            .map(|stream| {
                (
                    stream.target,
                    stream.semantic,
                    stream.values[index].map(f32::to_bits),
                )
            })
            .collect(),
    }
}

fn assert_reconstruction(asset: &ApplicationAsset, expected: &[Vec<CornerBits>]) -> Result<()> {
    assert_eq!(asset.draws.len(), expected.len());
    for (draw, corners) in asset.draws.iter().zip(expected) {
        assert_eq!(draw.indices.len(), corners.len());
        if let Some(rows) = &draw.influences {
            assert_eq!(rows.offsets.len(), draw.vertices.len() + 1);
            assert_eq!(rows.offsets.first(), Some(&0));
            assert_eq!(rows.offsets.last(), Some(&rows.joints.len()));
            assert_eq!(rows.weights.len(), rows.joints.len());
        }
        for stream in &draw.morphs {
            assert_eq!(stream.values.len(), draw.vertices.len());
        }
        let mut visited = vec![false; draw.vertices.len()];
        let mut next = 0;
        for (&index, corner) in draw.indices.iter().zip(corners) {
            let index = usize::try_from(index)?;
            assert!(index < draw.vertices.len());
            if !visited[index] {
                assert_eq!(index, next, "representatives follow first Corner encounter");
                visited[index] = true;
                next += 1;
            }
            assert_eq!(&owned_corner(draw, index), corner);
        }
        assert_eq!(next, draw.vertices.len());
    }
    Ok(())
}

fn assert_repeat(first: &ApplicationAsset, second: &ApplicationAsset) {
    assert_eq!(first.mesh_count, second.mesh_count);
    assert_eq!(first.draws.len(), second.draws.len());
    for (first, second) in first.draws.iter().zip(&second.draws) {
        assert_eq!(
            (first.mesh, first.primitive, first.material),
            (second.mesh, second.primitive, second.material)
        );
        assert_eq!(first.indices, second.indices);
        assert_eq!(first.vertices.len(), second.vertices.len());
        for index in 0..first.vertices.len() {
            assert_eq!(owned_corner(first, index), owned_corner(second, index));
        }
    }
}

const QUAD: &str = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0 0 0\n\
    vt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nvt 0.5 0\nvt -0 0\n\
    vn 0 0 1\nvn 0 1 0\nf 1/1/1 2/2/1 3/3/1\n";

fn process_obj(source: &str) -> Result<ApplicationAsset> {
    let report = Importer::new()?.read_bytes("indexing.obj", source.as_bytes())?;
    let expected = expanded_corners(report.scene())?;
    drop(report);
    let io = MemoryAssetIo::new();
    let path = AssetPath::new("indexing.obj")?;
    io.try_insert(path.clone(), Arc::from(source.as_bytes()))?;
    let options = Options {
        target: TargetSpace::STANDARD,
        source_assumptions: Some(TargetSpace::STANDARD),
        ..Options::default()
    };
    let first = process(&io, path.clone(), options)?.asset;
    let second = process(&io, path, options)?.asset;
    // Both process calls have destroyed their Scenes; the expected tuples are also owned.
    assert_reconstruction(&first, &expected)?;
    assert_repeat(&first, &second);
    Ok(first)
}

#[test]
fn quad_uses_four_vertices_and_six_indices_in_first_encounter_order() -> Result<()> {
    // Authored normals are preserved, so an unselected Face smoothing group is not a seam.
    let asset = process_obj(&format!("{QUAD}s 2\nf 1/1/1 3/3/1 4/4/1\n"))?;
    let draw = &asset.draws[0];
    assert_eq!(draw.vertices.len(), 4);
    assert_eq!(draw.indices, [0, 1, 2, 0, 2, 3]);
    Ok(())
}

#[test]
fn corner_uv_normal_and_signed_zero_seams_split_only_the_necessary_sources() -> Result<()> {
    for (face, indices) in [
        ("f 1/5/1 3/3/1 4/4/1\n", [0, 1, 2, 3, 2, 4]),
        ("f 1/6/1 3/3/1 4/4/1\n", [0, 1, 2, 3, 2, 4]),
        ("f 1/1/2 3/3/1 4/4/1\n", [0, 1, 2, 3, 2, 4]),
        ("f 1/5/1 3/3/2 4/4/1\n", [0, 1, 2, 3, 4, 5]),
    ] {
        let asset = process_obj(&format!("{QUAD}{face}"))?;
        let draw = &asset.draws[0];
        assert_eq!(draw.indices, indices, "{face}");
        let vertex_count = if indices[4] == 2 { 5 } else { 6 };
        assert_eq!(draw.vertices.len(), vertex_count, "{face}");
        if face.contains("1/6/1") {
            assert_eq!(
                draw.uv_stream(0).unwrap().values[0][0].to_bits(),
                0.0_f64.to_bits()
            );
            assert_eq!(
                draw.uv_stream(0).unwrap().values[3][0].to_bits(),
                (-0.0_f64).to_bits()
            );
        }
    }
    Ok(())
}

#[test]
fn coincident_positions_keep_distinct_source_vertex_identities() -> Result<()> {
    let asset = process_obj(&format!("{QUAD}f 5/1/1 3/3/1 4/4/1\n"))?;
    let draw = &asset.draws[0];
    assert_eq!(draw.vertices.len(), 5);
    assert_eq!(draw.indices, [0, 1, 2, 3, 2, 4]);
    assert_eq!(owned_corner(draw, 0), owned_corner(draw, 3));
    Ok(())
}

#[test]
fn identical_triangles_in_separate_draws_keep_independent_indices() -> Result<()> {
    let source = "v 0 0 0\nv 1 0 0\nv 0 1 0\nvn 0 0 1\n\
        o first\nf 1//1 2//1 3//1\no second\nf 1//1 2//1 3//1\n";
    let asset = process_obj(source)?;
    assert_eq!(asset.draws.len(), 2);
    for draw in &asset.draws {
        assert_eq!(draw.vertices.len(), 3);
        assert_eq!(draw.indices, [0, 1, 2]);
    }
    for index in 0..3 {
        assert_eq!(
            owned_corner(&asset.draws[0], index),
            owned_corner(&asset.draws[1], index)
        );
    }
    Ok(())
}

#[test]
fn upstream_corners_and_deformations_reconstruct_after_scene_drop() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let io = FileSystemAssetIo::from_capability(NativeDirectoryCapability::open_ambient(&root)?);
    let importer = Importer::new()?;
    let expectations: Value = serde_json::from_str(include_str!("../fixtures/expectations.json"))?;
    for fixture in expectations["fixtures"].as_array().unwrap() {
        if fixture["expected_outcome"] != "success" {
            continue;
        }
        let path = fixture["path"].as_str().unwrap();
        let report = importer.read_asset(&io, AssetPath::new(path)?)?;
        let (scene, _) = report.into_parts();
        // All successful upstream fixtures have authored normals, so the workflow preserves
        // them. Capture the former expanded tuples after the same explicit space conversion.
        let (scene, _) = convert_scene_space(
            scene,
            SpaceConversionOptions {
                target: Options::default().target,
                source_assumptions: None,
            },
            PostProcessControl::default(),
        )?
        .into_parts();
        let expected = expanded_corners(&scene)?;
        drop(scene);
        let first = process_file(&root, path, Options::default())?.asset;
        let second = process_file(&root, path, Options::default())?.asset;
        assert_reconstruction(&first, &expected)?;
        assert_repeat(&first, &second);
        if matches!(
            fixture["id"].as_str(),
            Some("BoxTextured" | "BoxInterleaved")
        ) {
            let draw = &first.draws[0];
            // Independent source accessor counts, also recorded in expectations.json.
            assert_eq!((draw.vertices.len(), draw.indices.len()), (24, 36));
        }
    }
    Ok(())
}

mod color_domains {
    use super::*;
    use baozi::{
        AttributeDomain, CapabilityStatus, Format, FormatCapability, FormatDescriptor,
        FormatEncoding, FormatId, FormatSelection, ImportConfigOverlay, ImportOperationControl,
        ImportRequest, ImportRequestLayer, MaterialAssignment, OptionCompileLimits,
    };
    use baozi_import::authoring::{
        AttributeSemantic, AuthoredTransform, AuthoringRange, GeometryAttributeAuthoring,
        GeometryLogicalMeshAuthoring, GeometryMeshInstanceAuthoring, GeometryPrimitiveAuthoring,
        GeometryTopologyAuthoring, LogicalMeshLocalId, NodeAuthoringNode, NodeLocalId, Presence,
        SceneSpace, StaticSceneBuilder, TangentBasisConvention, TangentConventions,
        TangentHandednessEncoding,
    };
    use baozi_import::{
        CompiledOptions, DetectionOutcome, DetectionResult, DetectionScope, NoFormatOptions,
        OptionCompileError, ParseContext, ParseResult, Probe, ReadConfidence, ReadHint,
        RegistryBuildScope,
    };

    #[derive(Clone, Copy)]
    enum ColorCase {
        CornerAlpha,
        CornerSignedZero,
        FaceRgba,
    }

    struct ColorImporter(ColorCase);

    impl Format for ColorImporter {
        type Options = NoFormatOptions;

        fn compile_defaults(
            &self,
            scope: &mut RegistryBuildScope<'_>,
        ) -> std::result::Result<CompiledOptions<Self::Options>, OptionCompileError> {
            NoFormatOptions::compile_defaults(scope)
        }

        fn descriptor(&self) -> FormatDescriptor {
            FormatDescriptor::builder(
                FormatId::new("org.baozi.test.color-domains").unwrap(),
                "Original color domain fixture",
            )
            .extensions(["fixture"])
            .encoding(FormatEncoding::Binary)
            .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
            .build()
            .unwrap()
        }

        fn detect(
            &self,
            probe: &Probe<'_>,
            _hint: &ReadHint,
            _scope: &mut DetectionScope<'_, '_>,
        ) -> DetectionResult<DetectionOutcome> {
            Ok(if probe.is_complete() && probe.bytes() == b"color" {
                DetectionOutcome::Match(ReadConfidence::Certain)
            } else {
                DetectionOutcome::NoMatch
            })
        }

        fn parse(
            &self,
            _options: &Self::Options,
            scope: &mut ParseContext<'_, '_>,
        ) -> ParseResult<()> {
            let positions = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
            let indices = [2, 0, 1, 1, 0, 3];
            let normals = [0.0, 0.0, 1.0].repeat(4);
            let tangents = [1.0, 0.0, 0.0, 1.0].repeat(4);
            let uvs = [0.25, 0.5].repeat(4);
            let mut colors = [[0.125, 0.5, 0.75, 1.0]; 6];
            let domain = match self.0 {
                ColorCase::CornerAlpha => {
                    colors[3][3] = 0.25;
                    AttributeDomain::Corner
                }
                ColorCase::CornerSignedZero => {
                    for color in &mut colors {
                        color[1] = 0.0;
                    }
                    colors[3][1] = -0.0;
                    AttributeDomain::Corner
                }
                ColorCase::FaceRgba => {
                    colors[1] = [0.875, 0.125, 0.625, 0.5];
                    AttributeDomain::Face
                }
            };
            let colors = if domain == AttributeDomain::Face {
                &colors[..2]
            } else {
                &colors[..]
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
                    AttributeSemantic::Normal,
                    AttributeDomain::Vertex,
                    3,
                    &normals,
                    Presence::Absent,
                ),
                GeometryAttributeAuthoring::f32(
                    AttributeSemantic::Tangent,
                    AttributeDomain::Vertex,
                    4,
                    &tangents,
                    Presence::Absent,
                )
                .with_tangent_conventions(TangentConventions {
                    basis: TangentBasisConvention::Unknown,
                    encoding: TangentHandednessEncoding::SignInW,
                }),
                GeometryAttributeAuthoring::f32(
                    AttributeSemantic::Texcoord(0),
                    AttributeDomain::Vertex,
                    2,
                    &uvs,
                    Presence::Absent,
                ),
                GeometryAttributeAuthoring::f32(
                    AttributeSemantic::Color(0),
                    domain,
                    4,
                    colors.as_flattened(),
                    Presence::Absent,
                ),
            ];
            let primitives = [GeometryPrimitiveAuthoring::new(
                GeometryTopologyAuthoring::TriangleList,
                Presence::Present(&indices),
                MaterialAssignment::Unspecified,
                AuthoringRange::new(0, 5),
            )];
            let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
            let roots = [NodeLocalId::new(0)];
            let nodes = [NodeAuthoringNode::new(
                Presence::Absent,
                AuthoredTransform::identity_trs(),
                Presence::Absent,
                Presence::Absent,
            )];
            let instances = [GeometryMeshInstanceAuthoring::new(
                NodeLocalId::new(0),
                LogicalMeshLocalId::new(0),
            )];
            scope.submit(
                StaticSceneBuilder::new(Presence::Present(&roots), &nodes)
                    .with_scene_space(&SceneSpace::STANDARD_TARGET)
                    .with_geometry(&meshes, &primitives, &attributes, &instances),
            )
        }
    }

    fn detached(case: ColorCase) -> Result<ApplicationAsset> {
        let mut builder = Importer::builder();
        builder.register(ColorImporter(case))?;
        let importer = builder.build()?;
        let input = ImportRequest::new().with_caller_layer(ImportRequestLayer::new(
            ImportConfigOverlay::default().with_format_selection(FormatSelection::Forced(
                FormatId::new("org.baozi.test.color-domains")?,
            )),
        ));
        let request = importer.plan(&input, OptionCompileLimits::default())?;
        let io = MemoryAssetIo::new();
        let fixture = vec![b'c', b'o', b'l', b'o', b'r'];
        io.try_insert(AssetPath::new("color.fixture")?, Arc::from(fixture.clone()))?;
        let report = request
            .read_asset(
                &io,
                AssetPath::new("color.fixture")?,
                ImportOperationControl::default(),
            )
            .map_err(baozi::Error::from)?;
        assert!(report.loss_evidence().is_empty());
        let (scene, evidence) = report.into_parts();
        let asset = ApplicationAsset::from_scene(&scene)?;
        drop(scene);
        drop(evidence);
        drop(request);
        drop(input);
        drop(importer);
        drop(io);
        drop(fixture);
        Ok(asset)
    }

    #[test]
    fn corner_face_colors_and_signed_zero_alone_control_reuse_after_source_drop() -> Result<()> {
        // Literal widened source RGBA bits, including a signed-zero-only distinction.
        const A: [u64; 4] = [
            0x3fc0000000000000,
            0x3fe0000000000000,
            0x3fe8000000000000,
            0x3ff0000000000000,
        ];
        const ALPHA: [u64; 4] = [
            0x3fc0000000000000,
            0x3fe0000000000000,
            0x3fe8000000000000,
            0x3fd0000000000000,
        ];
        const Z: [u64; 4] = [
            0x3fc0000000000000,
            0,
            0x3fe8000000000000,
            0x3ff0000000000000,
        ];
        const NEG_Z: [u64; 4] = [
            0x3fc0000000000000,
            0x8000000000000000,
            0x3fe8000000000000,
            0x3ff0000000000000,
        ];
        const B: [u64; 4] = [
            0x3fec000000000000,
            0x3fc0000000000000,
            0x3fe4000000000000,
            0x3fe0000000000000,
        ];
        // Authored source indices (2,0,1,1,0,3), independent of the tuple extractor.
        const P: [[f64; 3]; 6] = [
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
        ];
        for (case, indices, colors) in [
            (
                ColorCase::CornerAlpha,
                [0, 1, 2, 3, 1, 4],
                [A, A, A, ALPHA, A, A],
            ),
            (
                ColorCase::CornerSignedZero,
                [0, 1, 2, 3, 1, 4],
                [Z, Z, Z, NEG_Z, Z, Z],
            ),
            (ColorCase::FaceRgba, [0, 1, 2, 3, 4, 5], [A, A, A, B, B, B]),
        ] {
            let asset = detached(case)?;
            assert_eq!(asset.draws.len(), 1);
            let draw = &asset.draws[0];
            assert_eq!(draw.indices, indices);
            assert_eq!(
                draw.vertices.len(),
                if matches!(case, ColorCase::FaceRgba) {
                    6
                } else {
                    5
                }
            );
            for corner in 0..6 {
                let index = usize::try_from(draw.indices[corner])?;
                let vertex = &draw.vertices[index];
                assert_eq!(
                    vertex.position.map(f64::to_bits),
                    P[corner].map(f64::to_bits)
                );
                assert_eq!(
                    vertex.color.map(|v| v.map(f64::to_bits)),
                    Some(colors[corner])
                );
                assert_eq!(vertex.normal, Some([0.0, 0.0, 1.0]));
                assert_eq!(vertex.tangent, Some([1.0, 0.0, 0.0, 1.0]));
                assert_eq!(draw.uv_stream(0).unwrap().values[index], [0.25, 0.5]);
                assert_eq!(owned_corner(draw, index).color, Some(colors[corner]));
            }
        }
        Ok(())
    }
}
