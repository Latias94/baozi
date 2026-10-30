//! A downstream plugin using only parser authoring contracts and public Scene views.

use baozi::{AttributeData, AuthoredTransformKind, Importer, Mat4, SceneValidationError, Vec3};
use baozi_import::authoring::{
    AlphaMode, AttributeDomain, AttributeSemantic, AuthoredTransform, AuthoringRange,
    CameraLocalId, CameraProjection, Color, ColorInterpretation, GeometryAttributeAuthoring,
    GeometryLogicalMeshAuthoring, GeometryMeshInstanceAuthoring, GeometryPrimitiveAuthoring,
    GeometryTopologyAuthoring, ImageAuthoring, ImageLocalId, LightDefinition, LightKind,
    LightLocalId, LocalAxes, LogicalMeshLocalId, MaterialAuthoring, MaterialBindingAuthoring,
    MaterialFactors, NodeAuthoringNode, NodeLocalId, NodeParent, Presence, SamplerAuthoring,
    SamplerLocalId, SamplerState, ShadingModel, StaticSceneBuilder, SupplementalPbrFactors,
    TextureAuthoring, TextureBlendOperation, TextureChannelSwizzle, TextureLocalId, TextureRole,
    TextureSourceKey, TextureStrength, TextureTransform,
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
                Presence::Present(CameraLocalId::new(0)),
                Presence::Present(LightLocalId::new(0)),
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
        let cameras =
            [CameraProjection::orthographic(LocalAxes::STANDARD, 2.0, 3.0, 0.0, 10.0).unwrap()];
        let lights = [LightDefinition::try_from_scaled_linear_srgb(
            LightKind::spot(Some(5.0), 0.25, 0.5).unwrap(),
            LocalAxes::STANDARD,
            Color::WHITE,
            7.0,
        )
        .unwrap()];
        let images = [ImageAuthoring::encoded(
            &[0xde, 0xad, 0xbe, 0xef],
            Presence::Absent,
        )];
        let samplers = [SamplerAuthoring::new(SamplerState::CANONICAL_DEFAULT)];
        let textures = [TextureAuthoring::new(
            ImageLocalId::new(0),
            SamplerLocalId::new(0),
        )];
        let key = TextureSourceKey::try_new("custom:displacement").unwrap();
        let bindings = [MaterialBindingAuthoring::try_from_parts(
            TextureLocalId::new(0),
            TextureRole::Displacement,
            u16::MAX,
            TextureChannelSwizzle::RED,
            ColorInterpretation::Data,
            TextureTransform::IDENTITY,
            TextureStrength::try_new(-0.75).unwrap(),
            TextureBlendOperation::Replace,
            Some(&key),
        )
        .unwrap()];
        let factors = MaterialFactors::try_new_with_supplemental_pbr(
            Vec3::new(-0.0, 0.25, 0.5),
            None,
            None,
            None,
            SupplementalPbrFactors::try_new(
                0.125,
                -0.0,
                0.25,
                1.5,
                Vec3::new(0.25, 0.5, 0.75),
                0.5,
            )
            .unwrap(),
        )
        .unwrap();
        let materials = [MaterialAuthoring::try_from_parts(
            AuthoringRange::new(0, 1),
            ShadingModel::Unlit,
            AlphaMode::mask(-0.0).unwrap(),
            true,
            factors,
        )
        .unwrap()
        .with_name("lengths")];
        scope.submit_static_scene(
            StaticSceneBuilder::new(Presence::Present(&roots), &nodes)
                .with_scene_space(&baozi::SceneSpace::STANDARD_TARGET)
                .with_spatial_definitions(&cameras, &lights)
                .with_appearance(&images, &samplers, &textures, &bindings, &materials)
                .with_geometry(&meshes, &primitives, &attributes, &instances),
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
    } = first.normals(AttributeDomain::Vertex).unwrap().data()
    else {
        panic!("sparse normals must remain sparse")
    };
    assert_eq!((lanes, logical_count), (3, 3));
    assert_eq!(indices, &[1]);
    assert_eq!(values, &[0.0, 0.0, 1.0]);
    assert_eq!(default, &[0.0, 0.0, 0.0]);
    let CameraProjection::Orthographic(camera) = child.camera().unwrap().projection() else {
        panic!("custom orthographic camera");
    };
    assert_eq!(camera.axes(), LocalAxes::STANDARD);
    assert_eq!(camera.horizontal_magnification(), 2.0);
    assert_eq!(
        child.light().unwrap().definition().kind().range(),
        Some(5.0)
    );
    assert_appearance(scene, 1.5, -0.75);

    #[cfg(feature = "postprocess")]
    verify_spatial_conversion(
        importer
            .read_bytes("spatial.custom", b"valid")?
            .into_parts()
            .0,
    );

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

#[cfg(feature = "postprocess")]
fn verify_spatial_conversion(scene: baozi::ValidatedScene) {
    use baozi::{
        Axis, FrontFaceWinding, Handedness, TargetSpace,
        postprocess::{PostProcessControl, SpaceConversionOptions, convert_scene_space},
    };
    let count = scene.node_count();
    let image_ptr = scene.images().next().unwrap().encoded_bytes().as_ptr();
    let name_ptr = scene.materials().next().unwrap().name().unwrap().as_ptr();
    let key_ptr = scene
        .materials()
        .next()
        .unwrap()
        .bindings()
        .next()
        .unwrap()
        .source_key()
        .unwrap()
        .as_ptr();
    // Q(x,y,z)=(-x,-z,y), s=2; the optical reference frame changes, hierarchy does not.
    let target = TargetSpace::try_new(
        Handedness::Left,
        Axis::PositiveZ,
        Axis::PositiveY,
        0.5,
        FrontFaceWinding::CounterClockwise,
    )
    .unwrap();
    let (scene, report) = convert_scene_space(
        scene,
        SpaceConversionOptions {
            target,
            source_assumptions: None,
        },
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    assert!(report.matches_scene(&scene));
    assert_eq!(scene.node_count(), count);
    let child = scene.roots().nth(1).unwrap().children().next().unwrap();
    let axes = LocalAxes::try_new(Axis::NegativeX, Axis::PositiveZ, Axis::PositiveY).unwrap();
    let CameraProjection::Orthographic(camera) = child.camera().unwrap().projection() else {
        panic!("converted orthographic camera");
    };
    assert_eq!(camera.axes(), axes);
    assert_eq!(camera.horizontal_magnification(), 4.0);
    assert_eq!(camera.vertical_magnification(), 6.0);
    assert_eq!(camera.near(), 0.0);
    assert_eq!(camera.far(), 20.0);
    let light = child.light().unwrap().definition();
    assert_eq!(light.axes(), axes);
    assert_eq!(light.kind().range(), Some(10.0));
    assert_eq!(light.kind().inner_cone_radians(), Some(0.25));
    assert_eq!(light.kind().outer_cone_radians(), Some(0.5));
    assert_eq!(light.intensity(), 7.0);
    assert_appearance(&scene, 3.0, -1.5);
    assert_eq!(
        scene.images().next().unwrap().encoded_bytes().as_ptr(),
        image_ptr
    );
    assert_eq!(
        scene.materials().next().unwrap().name().unwrap().as_ptr(),
        name_ptr
    );
    assert_eq!(
        scene
            .materials()
            .next()
            .unwrap()
            .bindings()
            .next()
            .unwrap()
            .source_key()
            .unwrap()
            .as_ptr(),
        key_ptr
    );
    let (scene, inverse) = convert_scene_space(
        scene,
        SpaceConversionOptions::default(),
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    assert!(inverse.matches_scene(&scene));
    assert_eq!(scene.node_count(), count);
    assert_eq!(
        scene.cameras().next().unwrap().projection().axes(),
        LocalAxes::STANDARD
    );
    assert_eq!(
        scene.lights().next().unwrap().definition().axes(),
        LocalAxes::STANDARD
    );
    assert_appearance(&scene, 1.5, -0.75);
}

fn assert_appearance(scene: &baozi::ValidatedScene, thickness: f32, displacement: f32) {
    assert_eq!(
        scene.images().next().unwrap().encoded_bytes(),
        &[0xde, 0xad, 0xbe, 0xef]
    );
    let material = scene.materials().next().unwrap();
    assert_eq!(material.name(), Some("lengths"));
    assert_eq!(
        material.alpha_mode().mask_cutoff().unwrap().to_bits(),
        (-0.0_f32).to_bits()
    );
    assert!(material.double_sided());
    let factors = material.factors();
    assert!(factors.pbr_metallic_roughness().is_none());
    assert!(factors.pbr_specular_glossiness().is_none());
    assert!(factors.legacy().is_none());
    assert_eq!(factors.emissive().x.to_bits(), (-0.0_f32).to_bits());
    let supplemental = factors.supplemental_pbr();
    assert_eq!(supplemental.thickness().to_bits(), thickness.to_bits());
    assert_eq!(supplemental.clearcoat(), 0.125);
    assert_eq!(
        supplemental.clearcoat_roughness().to_bits(),
        (-0.0_f32).to_bits()
    );
    let binding = material.bindings().next().unwrap();
    assert_eq!(binding.role(), TextureRole::Displacement);
    assert_eq!(binding.uv_set(), u16::MAX);
    assert_eq!(binding.strength().get().to_bits(), displacement.to_bits());
    assert_eq!(binding.source_key(), Some("custom:displacement"));
}
