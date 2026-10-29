//! A downstream plugin using only parser authoring contracts and public Scene views.

use baozi::{AttributeData, AuthoredTransformKind, Importer, Mat4, SceneValidationError};
use baozi_import::authoring::{
    AttributeDomain, AttributeSemantic, AuthoredTransform, AuthoringRange,
    GeometryAttributeAuthoring, GeometryLogicalMeshAuthoring, GeometryMeshInstanceAuthoring,
    GeometryPrimitiveAuthoring, GeometryTopologyAuthoring, LogicalMeshLocalId, NodeAuthoringNode,
    NodeLocalId, NodeParent, Presence, StaticSceneBuilder,
};
use baozi_import::{
    CapabilityStatus, DetectionOutcome, DetectionResult, DetectionScope, FormatCapability,
    FormatDescriptor, FormatEncoding, FormatImporter, ParseResult, ParseScope, Probe,
    ReadConfidence, ReadHint, SceneDraft,
};

struct CustomScene;

impl FormatImporter for CustomScene {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            baozi::FormatId::new("dev.baozi.consumer.scene").unwrap(),
            "Independent Scene consumer",
        )
        .extensions(["custom"])
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
        .build()
        .unwrap()
    }

    fn detect(
        &self,
        _probe: &Probe<'_>,
        _hint: &ReadHint,
        _scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        let parent = if scope.primary_bytes() == b"bad-parent" {
            3
        } else {
            0
        };
        let roots = [NodeLocalId::new(2), NodeLocalId::new(0)];
        let nodes = [
            NodeAuthoringNode::new(
                Presence::Absent,
                AuthoredTransform::try_matrix(Mat4::IDENTITY).unwrap(),
                Presence::Absent,
                Presence::Absent,
            )
            .with_name("parent"),
            NodeAuthoringNode::new(
                Presence::Present(NodeParent::new(NodeLocalId::new(parent), 0)),
                AuthoredTransform::identity_trs(),
                Presence::Absent,
                Presence::Absent,
            ),
            NodeAuthoringNode::new(
                Presence::Absent,
                AuthoredTransform::identity_trs(),
                Presence::Absent,
                Presence::Absent,
            )
            .with_name(""),
        ];
        let positions = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let attributes = [
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                &positions,
                Presence::Absent,
            ),
            GeometryAttributeAuthoring::sparse_f32(
                AttributeSemantic::Normal,
                AttributeDomain::Vertex,
                3,
                3,
                &[1],
                &[0.0, 0.0, 1.0],
                &[0.0, 0.0, 0.0],
            ),
        ];
        let indices = [
            0,
            1,
            if scope.primary_bytes() == b"bad-index" {
                3
            } else {
                2
            },
        ];
        let primitives = [GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Present(&indices),
            Presence::Absent,
            AuthoringRange::new(0, 2),
        )];
        let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
        let instances = [GeometryMeshInstanceAuthoring::new(
            NodeLocalId::new(1),
            LogicalMeshLocalId::new(0),
        )];
        scope.submit_static_scene(
            StaticSceneBuilder::new(Presence::Present(&roots), &nodes).with_geometry(
                &meshes,
                &primitives,
                &attributes,
                &instances,
            ),
        )
    }
}

pub(super) fn verify() -> baozi::Result<()> {
    let mut builder = Importer::builder();
    builder.register(CustomScene)?;
    let importer = builder.build()?;
    let report = importer.read_bytes("hierarchy.custom", b"valid")?;
    let scene = report.scene();
    assert_eq!(scene.node_count(), 3);
    let roots: Vec<_> = scene.roots().collect();
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0].name(), Some(""));
    assert_eq!(roots[1].name(), Some("parent"));
    let child = roots[1].children().next().unwrap();
    assert_eq!(child.name(), None);
    assert_eq!(
        child.authored_transform().kind(),
        AuthoredTransformKind::Trs
    );
    assert_eq!(
        roots[1].authored_transform().kind(),
        AuthoredTransformKind::Matrix
    );
    assert_eq!(
        roots[1].authored_transform().authored_matrix(),
        Some(Mat4::IDENTITY)
    );
    assert_eq!(
        scene.mesh_instances().next().unwrap().node().id(),
        child.id()
    );
    let first = scene.meshes().next().unwrap().primitives().next().unwrap();
    let mut independent = scene.meshes();
    assert_eq!(
        independent
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap()
            .id(),
        first.id()
    );
    assert!(independent.next().is_none());
    let AttributeData::SparseF32 {
        lanes,
        logical_count,
        indices,
        values,
        default,
    } = first.normals().unwrap().data()
    else {
        panic!("sparse normals must remain sparse")
    };
    assert_eq!((lanes, logical_count), (3, 3));
    assert_eq!(indices, &[1]);
    assert_eq!(values, &[0.0, 0.0, 1.0]);
    assert_eq!(default, &[0.0, 0.0, 0.0]);

    for (bytes, expected) in [
        (
            b"bad-index".as_slice(),
            SceneValidationError::VertexIndexOutOfRange {
                primitive: 0,
                corner: 2,
                index: 3,
                vertex_count: 3,
            },
        ),
        (
            b"bad-parent".as_slice(),
            SceneValidationError::ParentOutOfRange {
                node: 1,
                parent: 3,
                node_count: 3,
            },
        ),
    ] {
        let error = importer.read_bytes("invalid.custom", bytes).unwrap_err();
        let baozi::Error::Import(failure) = error else {
            panic!("semantic submission failure expected: {error}")
        };
        assert_eq!(failure.scene_validation_error(), Some(&expected));
        assert_eq!(failure.source_identity().as_str(), "invalid.custom");
    }
    Ok(())
}
