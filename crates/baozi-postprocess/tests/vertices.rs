use baozi_core::{
    AttributeData, AttributeElement, AttributeSemanticRef, MorphDeltaSemantic, PrimitiveRef,
    SceneEntityKind, SceneLookupError, SceneSnapshotLimits, TopologyKind, Vec3,
};
use baozi_import::authoring::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, AttributeDomain, AttributeSemantic,
    AuthoredTransform, AuthoringRange, ContentDigest, CustomAttributeSemantic,
    DeformationAuthoringBatch, FormatId, GeometryAttributeAuthoring, GeometryAuthoringBatch,
    GeometryLogicalMeshAuthoring, GeometryMeshInstanceAuthoring, GeometryPrimitiveAuthoring,
    GeometryTopologyAuthoring, InfluenceAuthoring, InfluenceRowsAuthoring, LineageAuthoringBatch,
    LogicalMeshLocalId, LogicalMorphAuthoring, Mat4, MeshInstanceDeformationAuthoring,
    MetadataAuthoringBatch, MorphDeltaAuthoring, MorphDeltaPayloadAuthoring,
    MorphTargetDefinitionAuthoring, MorphTargetIdentity, NodeAuthoringBatch, NodeAuthoringNode,
    NodeLocalId, ObservedResourceAuthoring, OwnerProvenanceAuthoring, Presence,
    PrimitiveDeformationAuthoring, PrimitiveMorphTargetAuthoring, ProfileAuthoringBatch,
    ProfileOwner, ProvenanceLocalId, ProvenanceNodeAuthoring, ResourceIdentity,
    ResourceIdentityLocalId, ResourceSnapshotAuthoring, SceneAuthoringBatch, SceneSpace,
    SessionOpaqueReason, SkinAuthoring, SkinLocalId, SkinMatrixSpaces, SourceIdentityAuthoring,
    SourceIdentityLocalId, SourceObjectKind, SourceObjectLocator, SpatialAuthoringBatch,
    StaticSceneBuilder,
};
use baozi_import::{
    CapabilityStatus, DetectionOutcome, DetectionResult, DetectionScope, FormatCapability,
    FormatDescriptor, FormatEncoding, FormatImporter, FormatSelection, ImportConfigOverlay,
    ImportOperationControl, ImportReport, ImportRequest, ImportRequestLayer, OptionCompileLimits,
    ParseResult, ParseScope, ReadConfidence, ReadHint, RegistryBuildLimits, RegistryBuilder,
    SceneDraft, import_bytes,
};
use baozi_io::AssetPath;
use baozi_postprocess::{
    PrimitiveCorner, PrimitiveFace, PrimitiveVertex, TriangleVertices, VertexLayoutError,
    triangle_vertices,
};

const FORMAT: &str = "dev.baozi.test.triangle-vertices";
const PATH: &str = "fixtures/tuple.vertices";
const RICH_INDICES: [u32; 6] = [0, 1, 2, 3, 2, 1];
// Vertices 0 and 3 have identical positions; vertex 4 is deliberately unreferenced.
const RICH_POSITIONS: [f32; 15] = [
    0.0, -0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, -0.0, 0.0, 7.0, 8.0, 9.0,
];
const CORNER_NORMALS: [f32; 18] = [
    0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0,
];
const VERTEX_NORMALS: [f32; 15] = [
    1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0,
];
const VERTEX_UVS: [f32; 10] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 9.0, 9.0];
const FACE_COLORS: [f32; 8] = [-0.0, 0.25, 0.5, 1.0, 1.0, 0.5, 0.25, -0.0];
const FACE_F64: [f64; 4] = [16_777_217.0, -0.0, 1e100, f64::from_bits(1)];
const DENSE_I32: [i32; 5] = [i32::MIN, -2, 0, i32::MAX, 9];
const SPARSE_U32: [u32; 4] = [u32::MAX, 0x8000_0001, 7, 11];

#[derive(Debug, Clone, Copy)]
enum Fixture {
    Rich,
    Implicit,
    Empty { indexed: bool },
    Degenerate,
    Unsupported(GeometryTopologyAuthoring<'static>),
    Deformed,
}

struct TupleImporter(Fixture);

impl FormatImporter for TupleImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(FormatId::new(FORMAT).unwrap(), "Triangle tuple fixture")
            .encoding(FormatEncoding::Binary)
            .capabilities([
                (FormatCapability::Geometry, CapabilityStatus::Supported),
                (FormatCapability::Skinning, CapabilityStatus::Supported),
                (FormatCapability::MorphTargets, CapabilityStatus::Supported),
            ])
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
        match self.0 {
            Fixture::Rich => submit_rich(scope),
            Fixture::Deformed => submit_deformed(scope),
            fixture => submit_simple(scope, fixture),
        }
    }
}

fn custom(name: &str) -> AttributeSemantic {
    AttributeSemantic::Custom(CustomAttributeSemantic::new(name).unwrap())
}

fn submit_rich(scope: &ParseScope<'_>) -> ParseResult<SceneDraft> {
    let roots = [NodeLocalId::new(0)];
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
        Presence::Absent,
        Presence::Absent,
    )];
    let implicit_default = [-f32::from_bits(1)];
    let attributes = [
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &RICH_POSITIONS,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Normal,
            AttributeDomain::Vertex,
            3,
            &VERTEX_NORMALS,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Normal,
            AttributeDomain::Corner,
            3,
            &CORNER_NORMALS,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Texcoord(0),
            AttributeDomain::Vertex,
            2,
            &VERTEX_UVS,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::sparse_f32(
            AttributeSemantic::Texcoord(0),
            AttributeDomain::Corner,
            2,
            6,
            &[1, 4],
            &[0.5, -0.0, 0.75, 1.0],
            &[0.0, -0.0],
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Color(1),
            AttributeDomain::Face,
            4,
            &FACE_COLORS,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::u32(
            AttributeSemantic::SmoothingGroup,
            AttributeDomain::Face,
            1,
            &[0, 11],
        ),
        GeometryAttributeAuthoring::sparse_i32(
            custom("test:face-signed"),
            AttributeDomain::Face,
            2,
            2,
            &[1],
            &[i32::MIN, -5],
            &[-1, i32::MAX],
        ),
        GeometryAttributeAuthoring::sparse_f32(
            custom("test:implicit"),
            AttributeDomain::Vertex,
            1,
            5,
            &[],
            &[],
            &implicit_default,
        ),
        GeometryAttributeAuthoring::i32(
            custom("test:signed"),
            AttributeDomain::Vertex,
            1,
            &DENSE_I32,
        ),
        GeometryAttributeAuthoring::sparse_u32(
            custom("test:unsigned"),
            AttributeDomain::Corner,
            2,
            6,
            &[0, 5],
            &SPARSE_U32,
            &[99, u32::MAX],
        ),
        GeometryAttributeAuthoring::f64(custom("test:wide"), AttributeDomain::Face, 2, &FACE_F64),
    ];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::TriangleList,
        Presence::Present(&RICH_INDICES),
        Presence::Absent,
        AuthoringRange::new(0, 12),
    )];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let instances = [GeometryMeshInstanceAuthoring::new(
        NodeLocalId::new(0),
        LogicalMeshLocalId::new(0),
    )];
    scope.submit_static_scene(
        StaticSceneBuilder::new(Presence::Present(&roots), &nodes)
            .with_scene_space(&SceneSpace::STANDARD_TARGET)
            .with_geometry(&meshes, &primitives, &attributes, &instances),
    )
}

fn submit_simple(scope: &ParseScope<'_>, fixture: Fixture) -> ParseResult<SceneDraft> {
    let positions = [
        0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 2.0, 0.0, 0.0, 3.0, 0.0, 0.0, 2.0, 1.0, 0.0,
    ];
    let (topology, values, indices): (_, &[f32], Presence<&[u32]>) = match fixture {
        Fixture::Implicit => (
            GeometryTopologyAuthoring::TriangleList,
            &positions,
            Presence::Absent,
        ),
        Fixture::Empty { indexed } => (
            GeometryTopologyAuthoring::TriangleList,
            &[],
            if indexed {
                Presence::Present(&[])
            } else {
                Presence::Absent
            },
        ),
        Fixture::Degenerate => (
            GeometryTopologyAuthoring::TriangleList,
            &positions[..9],
            Presence::Present(&[0, 0, 1, 2, 2, 2]),
        ),
        Fixture::Unsupported(topology) => (topology, &positions, Presence::Absent),
        Fixture::Rich | Fixture::Deformed => {
            unreachable!("these fixtures have explicit submissions")
        }
    };
    let attributes = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        values,
        Presence::Absent,
    )];
    let primitives = [GeometryPrimitiveAuthoring::new(
        topology,
        indices,
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    scope.submit_static_scene(
        StaticSceneBuilder::new(Presence::Present(&[]), &[])
            .with_scene_space(&SceneSpace::STANDARD_TARGET)
            .with_geometry(&meshes, &primitives, &attributes, &[]),
    )
}

fn submit_deformed(scope: &ParseScope<'_>) -> ParseResult<SceneDraft> {
    let roots = [
        NodeLocalId::new(0),
        NodeLocalId::new(1),
        NodeLocalId::new(2),
        NodeLocalId::new(3),
    ];
    let nodes = roots.map(|_| {
        NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Absent,
            Presence::Absent,
        )
    });
    // Equal positions at vertices 0/1 must still address different influence and delta rows.
    let positions = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let attributes = std::array::from_fn::<_, 3, _>(|_| {
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &positions,
            Presence::Absent,
        )
    });
    let primitives = [
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Present(&[0, 1, 2, 0, 2, 1]),
            Presence::Absent,
            AuthoringRange::new(0, 1),
        ),
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Present(&[2, 1, 0]),
            Presence::Absent,
            AuthoringRange::new(1, 1),
        ),
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Absent,
            Presence::Absent,
            AuthoringRange::new(2, 1),
        ),
    ];
    let meshes = [
        GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 2)),
        GeometryLogicalMeshAuthoring::new(AuthoringRange::new(2, 1)),
    ];
    let instances = [
        GeometryMeshInstanceAuthoring::new(NodeLocalId::new(0), LogicalMeshLocalId::new(0)),
        GeometryMeshInstanceAuthoring::new(NodeLocalId::new(1), LogicalMeshLocalId::new(0)),
    ];
    let palette_a = [NodeLocalId::new(2), NodeLocalId::new(3)];
    let palette_b = [NodeLocalId::new(3), NodeLocalId::new(2)];
    let binds = [Mat4::IDENTITY; 2];
    let skins = [&palette_a, &palette_b].map(|joints| {
        SkinAuthoring::new(
            Presence::Absent,
            joints,
            &binds,
            Presence::Absent,
            SkinMatrixSpaces::canonical(),
        )
    });
    let definitions = [
        MorphTargetDefinitionAuthoring::new(
            MorphTargetIdentity::new(2),
            Presence::Present("dense"),
        ),
        MorphTargetDefinitionAuthoring::new(
            MorphTargetIdentity::new(0),
            Presence::Present("sparse"),
        ),
        MorphTargetDefinitionAuthoring::new(MorphTargetIdentity::new(3), Presence::Absent),
        MorphTargetDefinitionAuthoring::new(MorphTargetIdentity::new(1), Presence::Present("")),
        MorphTargetDefinitionAuthoring::new(
            MorphTargetIdentity::new(0),
            Presence::Present("other-mesh"),
        ),
    ];
    let logical_morphs = [
        LogicalMorphAuthoring::new(AuthoringRange::new(0, 4), &[-0.0, 0.25, 1.0, -0.5]),
        LogicalMorphAuthoring::new(AuthoringRange::new(4, 1), &[0.75]),
    ];
    let sparse_normal = [2.0, -0.0, f32::from_bits(1)];
    let dense_a = [
        MorphDeltaAuthoring::new(
            MorphDeltaSemantic::Position,
            MorphDeltaPayloadAuthoring::Dense {
                values: &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0],
            },
        ),
        MorphDeltaAuthoring::new(
            MorphDeltaSemantic::Normal,
            MorphDeltaPayloadAuthoring::Sparse {
                logical_count: 3,
                indices: &[1],
                values: &sparse_normal,
            },
        ),
    ];
    let sparse_a = [MorphDeltaAuthoring::new(
        MorphDeltaSemantic::Position,
        MorphDeltaPayloadAuthoring::Sparse {
            logical_count: 3,
            indices: &[0, 2],
            values: &[-0.0, 0.0, -0.0, 11.0, 12.0, 13.0],
        },
    )];
    let dense_b = [MorphDeltaAuthoring::new(
        MorphDeltaSemantic::Position,
        MorphDeltaPayloadAuthoring::Dense {
            values: &[
                101.0, 102.0, 103.0, 104.0, 105.0, 106.0, 107.0, 108.0, 109.0,
            ],
        },
    )];
    let sparse_b = [MorphDeltaAuthoring::new(
        MorphDeltaSemantic::Position,
        MorphDeltaPayloadAuthoring::Sparse {
            logical_count: 3,
            indices: &[1],
            values: &[21.0, 22.0, 23.0],
        },
    )];
    let empty_delta = [MorphDeltaAuthoring::new(
        MorphDeltaSemantic::Position,
        MorphDeltaPayloadAuthoring::Sparse {
            logical_count: 3,
            indices: &[],
            values: &[],
        },
    )];
    let other_delta = [MorphDeltaAuthoring::new(
        MorphDeltaSemantic::Position,
        MorphDeltaPayloadAuthoring::Dense {
            values: &[-1.0, -2.0, -3.0, -4.0, -5.0, -6.0, -7.0, -8.0, -9.0],
        },
    )];
    let targets_a = [
        PrimitiveMorphTargetAuthoring::new(&dense_a),
        PrimitiveMorphTargetAuthoring::new(&sparse_a),
        PrimitiveMorphTargetAuthoring::new(&[]),
        PrimitiveMorphTargetAuthoring::new(&empty_delta),
    ];
    let targets_b = [
        PrimitiveMorphTargetAuthoring::new(&dense_b),
        PrimitiveMorphTargetAuthoring::new(&sparse_b),
        PrimitiveMorphTargetAuthoring::new(&[]),
        PrimitiveMorphTargetAuthoring::new(&empty_delta),
    ];
    let targets_other = [PrimitiveMorphTargetAuthoring::new(&other_delta)];
    let influences = [
        InfluenceAuthoring::new(1, -0.0, 4),
        InfluenceAuthoring::new(0, 0.125, 3),
        InfluenceAuthoring::new(1, 0.25, 2),
        InfluenceAuthoring::new(0, 0.375, 1),
        InfluenceAuthoring::new(1, 0.25, 0),
        InfluenceAuthoring::new(0, 1.0, 0),
    ];
    let primitive_deformations = [
        PrimitiveDeformationAuthoring::new(
            Presence::Present(InfluenceRowsAuthoring::new(&[0, 5, 5, 6], &influences)),
            &targets_a,
        ),
        PrimitiveDeformationAuthoring::new(
            Presence::Present(InfluenceRowsAuthoring::new(&[0, 0, 0, 0], &[])),
            &targets_b,
        ),
        PrimitiveDeformationAuthoring::new(Presence::Absent, &targets_other),
    ];
    let instance_deformations = [
        MeshInstanceDeformationAuthoring::new(
            Presence::Present(SkinLocalId::new(0)),
            Presence::Present(&[1.0, -0.0, 0.5, -0.25]),
        ),
        MeshInstanceDeformationAuthoring::new(
            Presence::Present(SkinLocalId::new(1)),
            Presence::Present(&[-0.0, 0.75, 0.25, 1.0]),
        ),
    ];
    let format = FormatId::new(FORMAT).unwrap();
    let resource = ResourceIdentity::new(PATH).unwrap();
    let resources = [ObservedResourceAuthoring::session_opaque(
        &resource,
        SessionOpaqueReason::CallerLabel,
    )];
    let locator = SourceObjectLocator::indexed(SourceObjectKind::Document, 0);
    let sources =
        [
            SourceIdentityAuthoring::new(ResourceIdentityLocalId::new(0), &format, &locator)
                .with_content_digest(ContentDigest::sha256([
                    0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d,
                    0xae, 0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10,
                    0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
                ])),
        ];
    let provenance = [ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(
        0,
    ))];
    let attachment = [ProvenanceLocalId::new(0)];
    let mut owners = vec![ProfileOwner::Scene, ProfileOwner::SyntheticRoot];
    owners.extend((0..4).map(|index| ProfileOwner::Node(NodeLocalId::new(index))));
    owners.extend((0..2).map(ProfileOwner::LogicalMesh));
    owners.extend((0..3).map(ProfileOwner::Primitive));
    owners.extend((0..3).map(ProfileOwner::Attribute));
    owners.extend((0..2).map(ProfileOwner::MeshInstance));
    owners.extend((0..2).map(ProfileOwner::Skin));
    owners.extend((0..5).map(ProfileOwner::MorphDefinition));
    // Primitive 0 has four delta rows, primitive 1 three, and primitive 2 one.
    owners.extend((0..8).map(ProfileOwner::MorphDelta));
    let owners: Vec<_> = owners
        .into_iter()
        .map(|owner| OwnerProvenanceAuthoring::new(owner, &attachment))
        .collect();
    let snapshots = [ResourceSnapshotAuthoring::snapshotted(
        scope.primary_bytes(),
    )];
    scope.submit_scene(SceneAuthoringBatch::new(
        SpatialAuthoringBatch::new(&SceneSpace::STANDARD_TARGET, &[], &[]),
        NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
        AppearanceAuthoringBatch::empty(),
        GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &instances),
        DeformationAuthoringBatch::new(
            &skins,
            &definitions,
            &logical_morphs,
            &primitive_deformations,
            &instance_deformations,
        ),
        AnimationAuthoringBatch::empty(),
        ProfileAuthoringBatch::new(
            LineageAuthoringBatch::new(&resources, &sources, &provenance, &owners),
            MetadataAuthoringBatch::empty(),
        )
        .with_resource_snapshots(&snapshots),
    ))
}

fn import_fixture(fixture: Fixture) -> ImportReport {
    let mut registry = RegistryBuilder::new();
    registry.register(TupleImporter(fixture)).unwrap();
    let registry = registry.freeze(RegistryBuildLimits::default()).unwrap();
    let request = ImportRequest::new().with_caller_layer(ImportRequestLayer::new(
        ImportConfigOverlay::default()
            .with_format_selection(FormatSelection::Forced(FormatId::new(FORMAT).unwrap())),
    ));
    let request = registry
        .compile_request(&request, OptionCompileLimits::default())
        .unwrap();
    import_bytes(
        &request,
        AssetPath::new(PATH).unwrap(),
        b"abc",
        ImportOperationControl::default(),
    )
    .expect("tuple fixture must pass public central import validation")
}

fn f32_values(element: AttributeElement<'_>) -> &[f32] {
    let AttributeElement::F32(values) = element else {
        panic!("F32 element expected")
    };
    values
}

fn assert_f32(actual: &[f32], expected: &[f32]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert_eq!(actual.to_bits(), expected.to_bits());
    }
}

fn assert_vec3(actual: Vec3, expected: [f32; 3]) {
    assert_f32(&[actual.x, actual.y, actual.z], &expected);
}

fn first_primitive(report: &ImportReport) -> PrimitiveRef<'_> {
    report
        .scene()
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap()
}

#[test]
fn corners_expand_uv_seams_face_fields_and_simultaneous_semantic_domains_in_source_order() {
    let report = import_fixture(Fixture::Rich);
    let primitive = first_primitive(&report);
    let layout = triangle_vertices(primitive).unwrap();
    let layout_copy = layout;
    assert_eq!(layout.primitive().id(), primitive.id());
    assert_eq!(layout_copy.len(), 6);
    assert!(!layout.is_empty());
    assert_eq!(layout.indices(), 0..6);
    assert_eq!(primitive.vertex_count(), 5);
    let expected_keys = [
        (AttributeSemanticRef::Position, AttributeDomain::Vertex),
        (AttributeSemanticRef::Normal, AttributeDomain::Vertex),
        (AttributeSemanticRef::Normal, AttributeDomain::Corner),
        (AttributeSemanticRef::Texcoord(0), AttributeDomain::Vertex),
        (AttributeSemanticRef::Texcoord(0), AttributeDomain::Corner),
        (AttributeSemanticRef::Color(1), AttributeDomain::Face),
        (AttributeSemanticRef::SmoothingGroup, AttributeDomain::Face),
        (
            AttributeSemanticRef::Custom("test:face-signed"),
            AttributeDomain::Face,
        ),
        (
            AttributeSemanticRef::Custom("test:implicit"),
            AttributeDomain::Vertex,
        ),
        (
            AttributeSemanticRef::Custom("test:signed"),
            AttributeDomain::Vertex,
        ),
        (
            AttributeSemanticRef::Custom("test:unsigned"),
            AttributeDomain::Corner,
        ),
        (
            AttributeSemanticRef::Custom("test:wide"),
            AttributeDomain::Face,
        ),
    ];
    for (corner, vertex) in layout.vertices().enumerate() {
        let vertex_copy = vertex;
        assert_eq!(vertex_copy.primitive().id(), primitive.id());
        assert_eq!(vertex.index(), corner as u32);
        assert_eq!(
            vertex.source_vertex(),
            PrimitiveVertex {
                primitive: primitive.id(),
                index: RICH_INDICES[corner],
            }
        );
        assert_eq!(
            vertex.source_corner(),
            PrimitiveCorner {
                primitive: primitive.id(),
                index: corner as u32
            }
        );
        assert_eq!(
            vertex.source_face(),
            PrimitiveFace {
                primitive: primitive.id(),
                index: (corner / 3) as u32
            }
        );
        let fields = vertex.attributes();
        assert_eq!(fields.len(), expected_keys.len());
        for (field, (semantic, domain)) in fields.zip(expected_keys) {
            let copied = field;
            assert_eq!(copied.attribute().semantic(), semantic);
            assert_eq!(field.attribute().domain(), domain);
            let exact = vertex.attribute(semantic, domain).unwrap();
            assert_eq!(exact.element(), field.element());
        }
        assert_f32(
            f32_values(
                vertex
                    .attribute(AttributeSemanticRef::Normal, AttributeDomain::Vertex)
                    .unwrap()
                    .element(),
            ),
            &[1.0, 0.0, 0.0],
        );
        assert_f32(
            f32_values(
                vertex
                    .attribute(AttributeSemanticRef::Normal, AttributeDomain::Corner)
                    .unwrap()
                    .element(),
            ),
            if corner < 3 {
                &[0.0, 0.0, 1.0]
            } else {
                &[0.0, 1.0, 0.0]
            },
        );
        assert_f32(
            f32_values(
                vertex
                    .attribute(AttributeSemanticRef::Color(1), AttributeDomain::Face)
                    .unwrap()
                    .element(),
            ),
            if corner < 3 {
                &FACE_COLORS[..4]
            } else {
                &FACE_COLORS[4..]
            },
        );
        assert!(
            vertex
                .attribute(AttributeSemanticRef::Normal, AttributeDomain::Face)
                .is_none()
        );
        assert!(
            vertex
                .attribute(AttributeSemanticRef::Color(0), AttributeDomain::Face)
                .is_none()
        );
        assert!(
            vertex
                .attribute(AttributeSemanticRef::Position, AttributeDomain::Instance)
                .is_none()
        );
        assert!(vertex.influences().is_none());
    }
    let first = layout.vertex(0).unwrap();
    let equal_position = layout.vertex(3).unwrap();
    assert_eq!(
        first
            .attribute(AttributeSemanticRef::Position, AttributeDomain::Vertex)
            .unwrap()
            .element(),
        equal_position
            .attribute(AttributeSemanticRef::Position, AttributeDomain::Vertex)
            .unwrap()
            .element(),
    );
    assert_ne!(first.source_vertex(), equal_position.source_vertex());
    assert!(
        layout
            .vertices()
            .all(|vertex| vertex.source_vertex().index != 4)
    );
    let seam_a = layout.vertex(1).unwrap();
    let seam_b = layout.vertex(5).unwrap();
    assert_eq!(seam_a.source_vertex(), seam_b.source_vertex());
    assert_f32(
        f32_values(
            seam_a
                .attribute(AttributeSemanticRef::Texcoord(0), AttributeDomain::Corner)
                .unwrap()
                .element(),
        ),
        &[0.5, -0.0],
    );
    assert_f32(
        f32_values(
            seam_b
                .attribute(AttributeSemanticRef::Texcoord(0), AttributeDomain::Corner)
                .unwrap()
                .element(),
        ),
        &[0.0, -0.0],
    );
    assert_f32(
        f32_values(
            seam_a
                .attribute(AttributeSemanticRef::Texcoord(0), AttributeDomain::Vertex)
                .unwrap()
                .element(),
        ),
        &[1.0, 0.0],
    );
}

#[test]
fn scalar_kinds_sparse_defaults_and_explicit_values_borrow_original_bits_and_addresses() {
    let report = import_fixture(Fixture::Rich);
    let primitive = first_primitive(&report);
    let layout = triangle_vertices(primitive).unwrap();
    let AttributeData::SparseF32 {
        values: uv_values,
        default: uv_default,
        ..
    } = primitive
        .texcoords(0, AttributeDomain::Corner)
        .unwrap()
        .data()
    else {
        panic!("sparse corner UV fixture")
    };
    let AttributeData::SparseU32 {
        values: unsigned,
        default: unsigned_default,
        ..
    } = primitive
        .attribute(
            AttributeSemanticRef::Custom("test:unsigned"),
            AttributeDomain::Corner,
        )
        .unwrap()
        .data()
    else {
        panic!("sparse U32 fixture")
    };
    let AttributeData::SparseI32 {
        values: signed,
        default: signed_default,
        ..
    } = primitive
        .attribute(
            AttributeSemanticRef::Custom("test:face-signed"),
            AttributeDomain::Face,
        )
        .unwrap()
        .data()
    else {
        panic!("sparse I32 fixture")
    };
    let AttributeData::DenseF64 { values: wide, .. } = primitive
        .attribute(
            AttributeSemanticRef::Custom("test:wide"),
            AttributeDomain::Face,
        )
        .unwrap()
        .data()
    else {
        panic!("dense F64 fixture")
    };
    let expected_uvs = [
        [0.0, -0.0],
        [0.5, -0.0],
        [0.0, -0.0],
        [0.0, -0.0],
        [0.75, 1.0],
        [0.0, -0.0],
    ];
    for (corner, vertex) in layout.vertices().enumerate() {
        let uv = f32_values(
            vertex
                .attribute(AttributeSemanticRef::Texcoord(0), AttributeDomain::Corner)
                .unwrap()
                .element(),
        );
        assert_f32(uv, &expected_uvs[corner]);
        assert_eq!(
            uv.as_ptr(),
            match corner {
                1 => uv_values.as_ptr(),
                4 => uv_values[2..].as_ptr(),
                _ => uv_default.as_ptr(),
            }
        );
        let AttributeElement::U32(value) = vertex
            .attribute(
                AttributeSemanticRef::Custom("test:unsigned"),
                AttributeDomain::Corner,
            )
            .unwrap()
            .element()
        else {
            panic!("U32 element expected")
        };
        let expected = match corner {
            0 => &SPARSE_U32[..2],
            5 => &SPARSE_U32[2..],
            _ => &[99, u32::MAX],
        };
        assert_eq!(value, expected);
        assert_eq!(
            value.as_ptr(),
            match corner {
                0 => unsigned.as_ptr(),
                5 => unsigned[2..].as_ptr(),
                _ => unsigned_default.as_ptr(),
            }
        );
        let AttributeElement::I32(value) = vertex
            .attribute(
                AttributeSemanticRef::Custom("test:face-signed"),
                AttributeDomain::Face,
            )
            .unwrap()
            .element()
        else {
            panic!("I32 element expected")
        };
        assert_eq!(
            value,
            if corner < 3 {
                &[-1, i32::MAX]
            } else {
                &[i32::MIN, -5]
            }
        );
        assert_eq!(
            value.as_ptr(),
            if corner < 3 {
                signed_default.as_ptr()
            } else {
                signed.as_ptr()
            }
        );
        let AttributeElement::I32(value) = vertex
            .attribute(
                AttributeSemanticRef::Custom("test:signed"),
                AttributeDomain::Vertex,
            )
            .unwrap()
            .element()
        else {
            panic!("dense I32 element expected")
        };
        assert_eq!(value, &[DENSE_I32[RICH_INDICES[corner] as usize]]);
        let AttributeElement::U32(value) = vertex
            .attribute(AttributeSemanticRef::SmoothingGroup, AttributeDomain::Face)
            .unwrap()
            .element()
        else {
            panic!("dense U32 element expected")
        };
        assert_eq!(value, if corner < 3 { &[0] } else { &[11] });
        let AttributeElement::F64(value) = vertex
            .attribute(
                AttributeSemanticRef::Custom("test:wide"),
                AttributeDomain::Face,
            )
            .unwrap()
            .element()
        else {
            panic!("F64 element expected")
        };
        let start = if corner < 3 { 0 } else { 2 };
        assert_eq!(value.len(), 2);
        assert_eq!(value.as_ptr(), wide[start..].as_ptr());
        for (value, expected) in value.iter().zip(&FACE_F64[start..start + 2]) {
            assert_eq!(value.to_bits(), expected.to_bits());
        }
        let field = vertex
            .attribute(
                AttributeSemanticRef::Custom("test:implicit"),
                AttributeDomain::Vertex,
            )
            .unwrap();
        assert!(matches!(
            field.attribute().data(),
            AttributeData::SparseF32 {
                logical_count: 5,
                indices: [],
                values: [],
                ..
            }
        ));
        assert_f32(f32_values(field.element()), &[-f32::from_bits(1)]);
    }
}

#[test]
fn iterators_are_independent_exact_double_ended_and_checked_at_both_index_limits() {
    let report = import_fixture(Fixture::Rich);
    let layout = triangle_vertices(first_primitive(&report)).unwrap();
    assert!(layout.vertex(layout.len()).is_none());
    assert!(layout.vertex(usize::MAX).is_none());
    let mut forward = layout.vertices();
    assert_eq!(forward.len(), 6);
    assert_eq!(forward.next().unwrap().index(), 0);
    let mut independent = forward.clone();
    assert_eq!(forward.next_back().unwrap().index(), 5);
    assert_eq!(forward.len(), 4);
    assert_eq!(independent.next().unwrap().index(), 1);
    assert_eq!(independent.next_back().unwrap().index(), 5);
    assert_eq!(independent.len(), 3);
    assert_eq!(
        forward.map(|vertex| vertex.index()).collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );
    assert_eq!(
        layout
            .vertices()
            .rev()
            .map(|vertex| vertex.index())
            .collect::<Vec<_>>(),
        [5, 4, 3, 2, 1, 0]
    );
    let mut fields = layout.vertex(0).unwrap().attributes();
    assert_eq!(fields.len(), 12);
    assert_eq!(
        fields.next().unwrap().attribute().semantic(),
        AttributeSemanticRef::Position
    );
    let mut independent = fields.clone();
    assert_eq!(
        fields.next_back().unwrap().attribute().semantic(),
        AttributeSemanticRef::Custom("test:wide")
    );
    assert_eq!(fields.len(), 10);
    assert_eq!(
        independent.next().unwrap().attribute().domain(),
        AttributeDomain::Vertex
    );
    assert_eq!(
        independent.next().unwrap().attribute().domain(),
        AttributeDomain::Corner
    );
    assert_eq!(independent.len(), 9);
    assert!(format!("{layout:?}").contains("TriangleVertices"));
}

#[test]
fn implicit_empty_and_degenerate_triangle_lists_keep_all_draw_occurrences() {
    for fixture in [
        Fixture::Implicit,
        Fixture::Empty { indexed: false },
        Fixture::Empty { indexed: true },
        Fixture::Degenerate,
    ] {
        let report = import_fixture(fixture);
        let primitive = first_primitive(&report);
        let layout = triangle_vertices(primitive).unwrap();
        match fixture {
            Fixture::Implicit => {
                assert!(primitive.indices().is_none());
                assert_eq!(layout.indices(), 0..6);
                for (index, vertex) in layout.vertices().enumerate() {
                    assert_eq!(vertex.source_vertex().index, index as u32);
                    assert_eq!(vertex.source_corner().index, index as u32);
                    assert_eq!(vertex.source_face().index, (index / 3) as u32);
                    let position = f32_values(
                        vertex
                            .attribute(AttributeSemanticRef::Position, AttributeDomain::Vertex)
                            .unwrap()
                            .element(),
                    );
                    let expected = [
                        [0.0, 0.0, 0.0],
                        [1.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0],
                        [2.0, 0.0, 0.0],
                        [3.0, 0.0, 0.0],
                        [2.0, 1.0, 0.0],
                    ];
                    assert_f32(position, &expected[index]);
                }
            }
            Fixture::Empty { indexed } => {
                assert_eq!(primitive.indices(), indexed.then_some(&[][..]));
                assert_eq!(layout.len(), 0);
                assert!(layout.is_empty());
                assert_eq!(layout.indices(), 0..0);
                assert!(layout.vertex(0).is_none());
                assert_eq!(layout.vertices().len(), 0);
                assert!(layout.vertices().next_back().is_none());
            }
            Fixture::Degenerate => {
                assert_eq!(layout.len(), 6);
                assert_eq!(layout.indices(), 0..6);
                assert_eq!(
                    layout
                        .vertices()
                        .map(|vertex| vertex.source_vertex().index)
                        .collect::<Vec<_>>(),
                    [0, 0, 1, 2, 2, 2]
                );
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn unsupported_topologies_return_the_actual_typed_topology_without_allocating() {
    for (source, expected) in [
        (GeometryTopologyAuthoring::Points, TopologyKind::Points),
        (GeometryTopologyAuthoring::LineList, TopologyKind::LineList),
        (
            GeometryTopologyAuthoring::LineStrip,
            TopologyKind::LineStrip,
        ),
        (GeometryTopologyAuthoring::LineLoop, TopologyKind::LineLoop),
        (
            GeometryTopologyAuthoring::TriangleStrip,
            TopologyKind::TriangleStrip,
        ),
        (
            GeometryTopologyAuthoring::TriangleFan,
            TopologyKind::TriangleFan,
        ),
        (
            GeometryTopologyAuthoring::Polygons(&[3, 3]),
            TopologyKind::Polygons,
        ),
    ] {
        let report = import_fixture(Fixture::Unsupported(source));
        let primitive = first_primitive(&report);
        let allocations = allocation_counter::measure(|| {
            assert_eq!(
                triangle_vertices(primitive).unwrap_err(),
                VertexLayoutError::UnsupportedTopology { topology: expected }
            );
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(allocations.bytes_total, 0);
    }
}

#[test]
fn influences_and_morph_deltas_follow_source_vertices_and_the_current_primitive_projection() {
    let report = import_fixture(Fixture::Deformed);
    let mesh = report.scene().meshes().next().unwrap();
    let primitive_a = mesh.primitives().next().unwrap();
    let primitive_b = mesh.primitives().nth(1).unwrap();
    let a = triangle_vertices(primitive_a).unwrap();
    let b = triangle_vertices(primitive_b).unwrap();
    let targets: Vec<_> = mesh
        .morph_target_definitions()
        .map(|target| target.id())
        .collect();
    assert_eq!(targets.len(), 4);
    assert_eq!(
        primitive_a.morph_targets().next().unwrap().id(),
        primitive_b.morph_targets().next().unwrap().id()
    );
    let expected_sources = [0, 1, 2, 0, 2, 1];
    let expected_dense = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]];
    for (corner, vertex) in a.vertices().enumerate() {
        let source = expected_sources[corner];
        let row = vertex.influences().unwrap();
        assert_eq!(row.vertex(), source);
        match source {
            0 => {
                assert_eq!(
                    row.len(),
                    5,
                    "the reader must not impose a four-influence layout"
                );
                assert_eq!(row.joint_indices(), &[1, 0, 1, 0, 1]);
                assert_eq!(row.source_orders(), &[4, 3, 2, 1, 0]);
                assert_f32(row.weights(), &[-0.0, 0.125, 0.25, 0.375, 0.25]);
            }
            1 => assert!(row.is_empty()),
            2 => {
                assert_eq!(row.joint_indices(), &[0]);
                assert_f32(row.weights(), &[1.0]);
            }
            _ => unreachable!(),
        }
        assert_vec3(
            vertex
                .morph_delta(targets[0], MorphDeltaSemantic::Position)
                .unwrap()
                .unwrap(),
            expected_dense[source as usize],
        );
        assert_vec3(
            vertex
                .morph_delta(targets[0], MorphDeltaSemantic::Normal)
                .unwrap()
                .unwrap(),
            if source == 1 {
                [2.0, -0.0, f32::from_bits(1)]
            } else {
                [0.0, 0.0, 0.0]
            },
        );
        assert!(
            vertex
                .morph_delta(targets[0], MorphDeltaSemantic::Tangent)
                .unwrap()
                .is_none()
        );
        assert_vec3(
            vertex
                .morph_delta(targets[1], MorphDeltaSemantic::Position)
                .unwrap()
                .unwrap(),
            match source {
                0 => [-0.0, 0.0, -0.0],
                1 => [0.0, 0.0, 0.0],
                2 => [11.0, 12.0, 13.0],
                _ => unreachable!(),
            },
        );
        assert!(
            vertex
                .morph_delta(targets[2], MorphDeltaSemantic::Position)
                .unwrap()
                .is_none()
        );
        assert_vec3(
            vertex
                .morph_delta(targets[3], MorphDeltaSemantic::Position)
                .unwrap()
                .unwrap(),
            [0.0, 0.0, 0.0],
        );
    }
    let expected_other = [
        [107.0, 108.0, 109.0],
        [104.0, 105.0, 106.0],
        [101.0, 102.0, 103.0],
    ];
    for (index, vertex) in b.vertices().enumerate() {
        assert_eq!(vertex.source_vertex().index, [2, 1, 0][index]);
        assert!(vertex.influences().unwrap().is_empty());
        assert_vec3(
            vertex
                .morph_delta(targets[0], MorphDeltaSemantic::Position)
                .unwrap()
                .unwrap(),
            expected_other[index],
        );
        assert!(
            vertex
                .morph_delta(targets[0], MorphDeltaSemantic::Normal)
                .unwrap()
                .is_none()
        );
        assert_vec3(
            vertex
                .morph_delta(targets[1], MorphDeltaSemantic::Position)
                .unwrap()
                .unwrap(),
            if index == 1 {
                [21.0, 22.0, 23.0]
            } else {
                [0.0, 0.0, 0.0]
            },
        );
    }
    assert_ne!(
        a.vertex(0).unwrap().source_vertex(),
        a.vertex(1).unwrap().source_vertex()
    );
    assert_eq!(
        a.vertex(0)
            .unwrap()
            .attribute(AttributeSemanticRef::Position, AttributeDomain::Vertex)
            .unwrap()
            .element(),
        a.vertex(1)
            .unwrap()
            .attribute(AttributeSemanticRef::Position, AttributeDomain::Vertex)
            .unwrap()
            .element()
    );
    assert_eq!(
        a.vertex(0)
            .unwrap()
            .influences()
            .unwrap()
            .weights()
            .as_ptr(),
        a.vertex(3)
            .unwrap()
            .influences()
            .unwrap()
            .weights()
            .as_ptr()
    );
    let other = report
        .scene()
        .meshes()
        .nth(1)
        .unwrap()
        .primitives()
        .next()
        .unwrap();
    assert!(
        triangle_vertices(other)
            .unwrap()
            .vertices()
            .all(|vertex| vertex.influences().is_none())
    );
}

#[test]
fn target_qualification_and_instance_palettes_and_overrides_remain_independent() {
    let report = import_fixture(Fixture::Deformed);
    let foreign = import_fixture(Fixture::Deformed);
    let scene = report.scene();
    let mesh = scene.meshes().next().unwrap();
    let vertex = triangle_vertices(mesh.primitives().next().unwrap())
        .unwrap()
        .vertex(0)
        .unwrap();
    let foreign_target = foreign
        .scene()
        .meshes()
        .next()
        .unwrap()
        .morph_target_definitions()
        .next()
        .unwrap()
        .id();
    assert!(matches!(
        vertex.morph_delta(foreign_target, MorphDeltaSemantic::Tangent),
        Err(SceneLookupError::GenerationMismatch {
            kind: SceneEntityKind::MorphTarget,
            ..
        })
    ));
    let other_mesh = scene.meshes().nth(1).unwrap();
    let wrong_owner = other_mesh.morph_target_definitions().next().unwrap().id();
    assert!(
        matches!(vertex.morph_delta(wrong_owner, MorphDeltaSemantic::Tangent), Err(SceneLookupError::OwnerMismatch { kind: SceneEntityKind::MorphTarget, expected, actual }) if expected == mesh.id() && actual == other_mesh.id())
    );
    let other_vertex = triangle_vertices(other_mesh.primitives().next().unwrap())
        .unwrap()
        .vertex(1)
        .unwrap();
    assert_vec3(
        other_vertex
            .morph_delta(wrong_owner, MorphDeltaSemantic::Position)
            .unwrap()
            .unwrap(),
        [-4.0, -5.0, -6.0],
    );
    let a = scene.mesh_instances().next().unwrap();
    let b = scene.mesh_instances().nth(1).unwrap();
    assert_eq!(a.mesh().id(), b.mesh().id());
    assert_eq!(a.mesh().id(), vertex.primitive().mesh().id());
    let joint_a = scene.nodes().nth(2).unwrap().id();
    let joint_b = scene.nodes().nth(3).unwrap().id();
    assert_eq!(
        a.skin().unwrap().joints().collect::<Vec<_>>(),
        [joint_a, joint_b]
    );
    assert_eq!(
        b.skin().unwrap().joints().collect::<Vec<_>>(),
        [joint_b, joint_a]
    );
    assert_f32(a.morph_weights().unwrap(), &[1.0, -0.0, 0.5, -0.25]);
    assert_f32(b.morph_weights().unwrap(), &[-0.0, 0.75, 0.25, 1.0]);
    assert_f32(mesh.default_morph_weights(), &[-0.0, 0.25, 1.0, -0.5]);
    // One immutable primitive layout is reusable by both placements: no chosen palette/weight
    // is baked into its source-relative deformer data.
    assert_eq!(
        vertex.influences().unwrap().joint_indices(),
        &[1, 0, 1, 0, 1]
    );
    assert_vec3(
        vertex
            .morph_delta(
                mesh.morph_target_definitions().next().unwrap().id(),
                MorphDeltaSemantic::Position,
            )
            .unwrap()
            .unwrap(),
        [1.0, 2.0, 3.0],
    );
}

fn position_after_temporary_layout<'scene>(primitive: PrimitiveRef<'scene>) -> &'scene [f32] {
    let temporary = triangle_vertices(primitive).unwrap();
    let mut vertices = temporary.vertices();
    let vertex = vertices.nth(1).unwrap();
    let field = vertex
        .attribute(AttributeSemanticRef::Position, AttributeDomain::Vertex)
        .unwrap();
    f32_values(field.element())
}

#[test]
fn elements_influences_and_iterators_keep_scene_lifetimes_after_temporary_views_drop() {
    let report = import_fixture(Fixture::Rich);
    let primitive = first_primitive(&report);
    let positions = position_after_temporary_layout(primitive);
    assert_f32(positions, &[1.0, 0.0, 0.0]);
    let AttributeData::DenseF32 { values, .. } = primitive.positions().unwrap().data() else {
        panic!("dense position fixture")
    };
    assert_eq!(positions.as_ptr(), values[3..].as_ptr());
    let fields = {
        triangle_vertices(primitive)
            .unwrap()
            .vertex(0)
            .unwrap()
            .attributes()
    };
    assert_eq!(fields.len(), 12);
    let AttributeElement::F64(wide) = fields.clone().next_back().unwrap().element() else {
        panic!("F64 field fixture")
    };
    assert_eq!(wide[0].to_bits(), 16_777_217.0_f64.to_bits());
    assert_eq!(wide[1].to_bits(), (-0.0_f64).to_bits());
    let deformed = import_fixture(Fixture::Deformed);
    let row = {
        triangle_vertices(first_primitive(&deformed))
            .unwrap()
            .vertex(0)
            .unwrap()
            .influences()
            .unwrap()
    };
    assert_eq!(row.len(), 5);
    assert_f32(row.weights(), &[-0.0, 0.125, 0.25, 0.375, 0.25]);
}

fn visit_all_fields(layout: TriangleVertices<'_>) {
    for vertex in layout.vertices() {
        for field in vertex.attributes() {
            std::hint::black_box((field.attribute(), field.element()));
        }
        std::hint::black_box((
            vertex.source_vertex(),
            vertex.source_corner(),
            vertex.source_face(),
        ));
        if let Some(row) = vertex.influences() {
            std::hint::black_box((row.joint_indices(), row.weights(), row.source_orders()));
        }
        for target in vertex.primitive().morph_targets() {
            for semantic in [
                MorphDeltaSemantic::Position,
                MorphDeltaSemantic::Normal,
                MorphDeltaSemantic::Tangent,
            ] {
                std::hint::black_box(vertex.morph_delta(target.id(), semantic).unwrap());
            }
        }
    }
}

#[test]
fn construction_random_access_and_complete_tuple_deformer_reads_allocate_nothing() {
    for fixture in [Fixture::Rich, Fixture::Deformed] {
        let report = import_fixture(fixture);
        let scene = report.scene();
        let limits = SceneSnapshotLimits::try_new(32 * 1024, 128, 1_000_000).unwrap();
        let before = scene.capture_snapshot(limits).unwrap();
        let generation = scene.generation();
        let allocations = allocation_counter::measure(|| {
            for mesh in scene.meshes() {
                for primitive in mesh.primitives() {
                    let layout = triangle_vertices(primitive).unwrap();
                    for _ in 0..8 {
                        std::hint::black_box(triangle_vertices(primitive).unwrap());
                        std::hint::black_box(layout.vertex(0));
                        std::hint::black_box(layout.vertex(usize::MAX));
                    }
                    visit_all_fields(layout);
                }
            }
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(allocations.bytes_total, 0);
        assert_eq!(allocations.count_current, 0);
        assert_eq!(allocations.bytes_current, 0);
        assert_eq!(scene.generation(), generation);
        assert_eq!(scene.capture_snapshot(limits).unwrap(), before);
        assert!(report.evidence().matches_scene(scene));
    }
}
