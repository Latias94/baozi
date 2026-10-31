use baozi_core::{
    AnimationTarget, AttributeData, CameraProjectionError, Mat4, NumericConversionError,
    TargetSpace, ValidatedScene,
};
use baozi_import::authoring::{
    AnimationAuthoringBatch, AnimationChannelAuthoring, AnimationClipAuthoring,
    AnimationExtrapolation, AnimationInterpolation, AnimationTargetAuthoring, AnimationTimeDomain,
    AppearanceAuthoringBatch, AttributeDomain, AttributeSemantic, AuthoredTransform,
    AuthoringRange, Axis, CameraLocalId, CameraProjection, Color, ContentDigest,
    DeformationAuthoringBatch, FormatId, FrontFaceWinding, GeometryAttributeAuthoring,
    GeometryAuthoringBatch, GeometryLogicalMeshAuthoring, GeometryMeshInstanceAuthoring,
    GeometryPrimitiveAuthoring, GeometryTopologyAuthoring, Handedness, LightDefinition, LightKind,
    LightLocalId, LineageAuthoringBatch, LocalAxes, LogicalMeshLocalId, LogicalMorphAuthoring,
    MeshInstanceDeformationAuthoring, MetadataAuthoringBatch, NodeAuthoringBatch,
    NodeAuthoringNode, NodeLocalId, NodeParent, ObservedResourceAuthoring,
    OwnerProvenanceAuthoring, Presence, PrimitiveDeformationAuthoring, ProfileAuthoringBatch,
    ProfileOwner, ProvenanceLocalId, ProvenanceNodeAuthoring, Quat, ResourceIdentity,
    ResourceIdentityLocalId, ResourceSnapshotAuthoring, SceneAuthoringBatch, SceneSpace,
    SessionOpaqueReason, SourceIdentityAuthoring, SourceIdentityLocalId, SourceObjectKind,
    SourceObjectLocator, SpatialAuthoringBatch, Vec3,
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
    PostProcessControl, PostProcessResourceEvidence, PostProcessRunDisposition,
    PostProcessRunError, SpaceConversionCause, SpaceConversionIssue, SpaceConversionOptions,
    convert_scene_space,
};

const FORMAT: &str = "dev.baozi.test.space-camera-light";
const PATH: &str = "fixtures/camera-light.spacefixture";
// SHA-256 of the independently observed primary bytes, b"abc".
const SOURCE_DIGEST: [u8; 32] = [
    0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
    0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
];
const POSITIONS: [f32; 9] = [1.0, 2.0, 3.0, 2.0, 2.0, 3.0, 1.0, 3.0, 3.0];
const NAMES: [&str; 6] = [
    "mixed",
    "point",
    "spot",
    "sun-copy",
    "point-copy",
    "spot-copy",
];

#[derive(Debug, Clone, Copy)]
enum Fixture {
    Valid,
    PerspectiveOverflow,
    PerspectiveUnderflow,
    OrthographicOverflow,
    OrthographicUnderflow,
    PerspectiveCollapsed,
    OrthographicCollapsed,
    PointOverflow,
    SpotUnderflow,
}

fn unusual_axes() -> LocalAxes {
    LocalAxes::try_new(Axis::PositiveY, Axis::NegativeZ, Axis::NegativeX).unwrap()
}

fn reflected_axes() -> LocalAxes {
    LocalAxes::try_new(Axis::NegativeX, Axis::PositiveY, Axis::NegativeZ).unwrap()
}

fn cameras(fixture: Fixture) -> [CameraProjection; 2] {
    let tiny = f32::from_bits(1);
    let next_normal = f32::from_bits(f32::MIN_POSITIVE.to_bits() + 1);
    let (near, far) = match fixture {
        Fixture::PerspectiveOverflow => (0.125, f32::MAX),
        Fixture::PerspectiveUnderflow => (tiny, 64.0),
        Fixture::PerspectiveCollapsed => (f32::MIN_POSITIVE, next_normal),
        _ => (0.125, 64.0),
    };
    let (horizontal, near_ortho, far_ortho) = match fixture {
        Fixture::OrthographicOverflow => (f32::MAX, 0.0, 32.0),
        Fixture::OrthographicUnderflow => (tiny, 0.0, 32.0),
        Fixture::OrthographicCollapsed => (2.0, f32::MIN_POSITIVE, next_normal),
        _ => (2.0, 0.0, 32.0),
    };
    [
        CameraProjection::perspective(
            LocalAxes::STANDARD,
            f32::from_bits(0x3f80_0001),
            Some(f32::from_bits(0x3fc0_0001)),
            near,
            Some(far),
        )
        .unwrap(),
        CameraProjection::orthographic(unusual_axes(), horizontal, 3.0, near_ortho, far_ortho)
            .unwrap(),
    ]
}

fn lights(fixture: Fixture) -> [LightDefinition; 3] {
    let point_range = if matches!(fixture, Fixture::PointOverflow) {
        f32::MAX
    } else {
        4.0
    };
    let spot_range = if matches!(fixture, Fixture::SpotUnderflow) {
        f32::from_bits(1)
    } else {
        16.0
    };
    [
        LightDefinition::try_from_scaled_linear_srgb(
            LightKind::directional(),
            LocalAxes::STANDARD,
            Color::WHITE,
            f32::MAX,
        )
        .unwrap(),
        LightDefinition::try_from_scaled_linear_srgb(
            LightKind::point(Some(point_range)).unwrap(),
            unusual_axes(),
            Color::linear_rgba(1.0, 0.0, 0.0, 1.0),
            1.0,
        )
        .unwrap(),
        LightDefinition::try_from_scaled_linear_srgb(
            LightKind::spot(Some(spot_range), -0.0, f32::from_bits(0x3f40_0001)).unwrap(),
            reflected_axes(),
            Color::linear_rgba(2.0, 1.0, 0.5, 1.0),
            4.0,
        )
        .unwrap(),
    ]
}

struct CameraLightImporter {
    fixture: Fixture,
    format: FormatId,
    resource: ResourceIdentity,
}

impl FormatImporter for CameraLightImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(self.format.clone(), "Camera and light space fixture")
            .encoding(FormatEncoding::Binary)
            .capabilities([
                (FormatCapability::Geometry, CapabilityStatus::Supported),
                (FormatCapability::CamerasLights, CapabilityStatus::Supported),
                (FormatCapability::Animation, CapabilityStatus::Supported),
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
        let cameras = cameras(self.fixture);
        let lights = lights(self.fixture);
        let roots = [NodeLocalId::new(0)];
        let parents = [
            None,
            Some((0, 0)),
            Some((0, 1)),
            Some((1, 0)),
            Some((1, 1)),
            Some((2, 0)),
        ];
        let nodes = std::array::from_fn::<_, 6, _>(|index| {
            let parent = parents[index].map_or(Presence::Absent, |(parent, order)| {
                Presence::Present(NodeParent::new(NodeLocalId::new(parent), order))
            });
            let transform = if index == 0 {
                AuthoredTransform::try_trs(
                    Vec3::new(1.0, 2.0, 3.0),
                    Quat::IDENTITY,
                    Vec3::new(1.0, 1.0, 1.0),
                )
                .unwrap()
            } else {
                AuthoredTransform::identity_trs()
            };
            NodeAuthoringNode::new(
                parent,
                transform,
                Presence::Present(CameraLocalId::new((index % 2) as u32)),
                Presence::Present(LightLocalId::new((index % 3) as u32)),
            )
            .with_name(NAMES[index])
        });
        let attributes = [GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &POSITIONS,
            Presence::Absent,
        )];
        let primitives = [GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Present(&[0, 1, 2]),
            Presence::Absent,
            AuthoringRange::new(0, 1),
        )];
        let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
        let instances = [GeometryMeshInstanceAuthoring::new(
            NodeLocalId::new(0),
            LogicalMeshLocalId::new(0),
        )];
        let morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
        let primitive_deformation = [PrimitiveDeformationAuthoring::new(Presence::Absent, &[])];
        let instance_deformation = [MeshInstanceDeformationAuthoring::new(
            Presence::Absent,
            Presence::Absent,
        )];
        let channels = [AnimationChannelAuthoring::new(
            0,
            AnimationTargetAuthoring::NodeTranslation(NodeLocalId::new(0)),
            AnimationInterpolation::Linear,
            AnimationExtrapolation::Constant,
            AnimationExtrapolation::Repeat,
            &[0.0, 1.0],
            &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            Presence::Absent,
            Presence::Absent,
        )];
        let clips = [AnimationClipAuthoring::new(
            0,
            1.0,
            AnimationTimeDomain::Seconds,
            AuthoringRange::new(0, 1),
        )
        .with_name("placement")];
        let resources = [ObservedResourceAuthoring::session_opaque(
            &self.resource,
            SessionOpaqueReason::CallerLabel,
        )];
        let locator = SourceObjectLocator::indexed(SourceObjectKind::Document, 0);
        let sources =
            [
                SourceIdentityAuthoring::new(
                    ResourceIdentityLocalId::new(0),
                    &self.format,
                    &locator,
                )
                .with_content_digest(ContentDigest::sha256(SOURCE_DIGEST)),
            ];
        let provenance = [ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(
            0,
        ))];
        let attachment = [ProvenanceLocalId::new(0)];
        let mut owners = vec![ProfileOwner::Scene, ProfileOwner::SyntheticRoot];
        owners.extend((0..6).map(|index| ProfileOwner::Node(NodeLocalId::new(index))));
        owners.extend((0..2).map(ProfileOwner::Camera));
        owners.extend((0..3).map(ProfileOwner::Light));
        owners.extend([
            ProfileOwner::LogicalMesh(0),
            ProfileOwner::Primitive(0),
            ProfileOwner::Attribute(0),
            ProfileOwner::MeshInstance(0),
            ProfileOwner::AnimationClip(0),
            ProfileOwner::AnimationChannel(0),
        ]);
        let owners: Vec<_> = owners
            .into_iter()
            .map(|owner| OwnerProvenanceAuthoring::new(owner, &attachment))
            .collect();
        let snapshots = [ResourceSnapshotAuthoring::snapshotted(
            scope.primary_bytes(),
        )];
        scope.submit_scene(SceneAuthoringBatch::new(
            SpatialAuthoringBatch::new(&SceneSpace::STANDARD_TARGET, &cameras, &lights),
            NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
            AppearanceAuthoringBatch::empty(),
            GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &instances),
            DeformationAuthoringBatch::new(
                &[],
                &[],
                &morphs,
                &primitive_deformation,
                &instance_deformation,
            ),
            AnimationAuthoringBatch::new(&clips, &channels),
            ProfileAuthoringBatch::new(
                LineageAuthoringBatch::new(&resources, &sources, &provenance, &owners),
                MetadataAuthoringBatch::empty(),
            )
            .with_resource_snapshots(&snapshots),
        ))
    }
}

fn import_fixture(fixture: Fixture) -> ImportReport {
    let mut registry = RegistryBuilder::new();
    registry
        .register(CameraLightImporter {
            fixture,
            format: FormatId::new(FORMAT).unwrap(),
            resource: ResourceIdentity::new(PATH).unwrap(),
        })
        .unwrap();
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
    .expect("public camera/light fixture must pass central import validation")
}

fn options(units: f64) -> SpaceConversionOptions {
    // Handwritten standard-source map: Q(x,y,z)=(-z,x,y), det(Q)=-1, s=1/units.
    SpaceConversionOptions {
        target: TargetSpace::try_new(
            Handedness::Left,
            Axis::PositiveZ,
            Axis::PositiveX,
            units,
            FrontFaceWinding::CounterClockwise,
        )
        .unwrap(),
        source_assumptions: None,
    }
}

fn assert_bits(actual: &[f32], expected: &[f32]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert_eq!(actual.to_bits(), expected.to_bits());
    }
}

fn affine_point(matrix: Mat4, point: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|row| {
        matrix.cols[0][row] * point[0]
            + matrix.cols[1][row] * point[1]
            + matrix.cols[2][row] * point[2]
            + matrix.cols[3][row]
    })
}

fn assert_camera_bits(actual: CameraProjection, expected: CameraProjection) {
    assert_eq!(actual.axes(), expected.axes());
    match (actual, expected) {
        (CameraProjection::Perspective(actual), CameraProjection::Perspective(expected)) => {
            assert_bits(
                &[actual.vertical_fov_radians(), actual.near()],
                &[expected.vertical_fov_radians(), expected.near()],
            );
            assert_eq!(
                actual.aspect_ratio().map(f32::to_bits),
                expected.aspect_ratio().map(f32::to_bits)
            );
            assert_eq!(
                actual.far().map(f32::to_bits),
                expected.far().map(f32::to_bits)
            );
        }
        (CameraProjection::Orthographic(actual), CameraProjection::Orthographic(expected)) => {
            assert_bits(
                &[
                    actual.horizontal_magnification(),
                    actual.vertical_magnification(),
                    actual.near(),
                    actual.far(),
                ],
                &[
                    expected.horizontal_magnification(),
                    expected.vertical_magnification(),
                    expected.near(),
                    expected.far(),
                ],
            );
        }
        _ => panic!("projection variant changed"),
    }
}

fn assert_light_bits(actual: LightDefinition, expected: LightDefinition) {
    assert_eq!(actual.axes(), expected.axes());
    assert_eq!(actual.kind().category(), expected.kind().category());
    assert_eq!(
        actual.kind().range().map(f32::to_bits),
        expected.kind().range().map(f32::to_bits)
    );
    assert_eq!(
        actual.kind().inner_cone_radians().map(f32::to_bits),
        expected.kind().inner_cone_radians().map(f32::to_bits)
    );
    assert_eq!(
        actual.kind().outer_cone_radians().map(f32::to_bits),
        expected.kind().outer_cone_radians().map(f32::to_bits)
    );
    let actual_tint = actual.tint();
    let expected_tint = expected.tint();
    assert_bits(
        &[
            actual_tint.x,
            actual_tint.y,
            actual_tint.z,
            actual.intensity(),
        ],
        &[
            expected_tint.x,
            expected_tint.y,
            expected_tint.z,
            expected.intensity(),
        ],
    );
}

fn positions(scene: &ValidatedScene) -> &[f32] {
    let AttributeData::DenseF32 { lanes: 3, values } = scene
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap()
        .positions()
        .unwrap()
        .data()
    else {
        panic!("dense XYZ positions expected");
    };
    values
}

fn assert_structure(scene: &ValidatedScene) {
    assert_eq!((scene.node_count(), scene.root_count()), (6, 1));
    assert_eq!(
        (
            scene.camera_count(),
            scene.light_count(),
            scene.mesh_instance_count()
        ),
        (2, 3, 1)
    );
    let node_ids: Vec<_> = scene.nodes().map(|node| node.id()).collect();
    assert_eq!(
        scene.roots().map(|node| node.id()).collect::<Vec<_>>(),
        &node_ids[..1],
    );
    let children = [
        &node_ids[1..3],
        &node_ids[3..5],
        &node_ids[5..6],
        &[],
        &[],
        &[],
    ];
    for (index, node) in scene.nodes().enumerate() {
        assert_eq!(node.name(), Some(NAMES[index]));
        assert_eq!(
            node.children().map(|child| child.id()).collect::<Vec<_>>(),
            children[index]
        );
        let camera = node.camera().unwrap();
        let light = node.light().unwrap();
        assert_eq!(camera.id(), scene.cameras().nth(index % 2).unwrap().id());
        assert_eq!(light.id(), scene.lights().nth(index % 3).unwrap().id());
        assert_eq!(
            scene.camera(camera.id()).unwrap().projection(),
            camera.projection()
        );
        assert_eq!(
            scene.light(light.id()).unwrap().definition(),
            light.definition()
        );
    }
    let root = scene.roots().next().unwrap();
    assert_eq!(
        scene.mesh_instances().next().unwrap().node().id(),
        root.id()
    );
    assert_eq!(scene.animation_clip_count(), 1);
    let clip = scene.animation_clips().next().unwrap();
    assert_eq!(clip.name(), Some("placement"));
    assert_eq!(clip.duration().to_bits(), 1.0_f64.to_bits());
    assert_eq!(clip.time_domain(), AnimationTimeDomain::Seconds);
    assert_eq!(clip.channel_count(), 1);
    let channel = clip.channels().next().unwrap();
    assert_eq!(
        channel.target(),
        AnimationTarget::NodeTranslation(root.id())
    );
    assert_eq!(channel.key_times(), &[0.0, 1.0]);
    assert_eq!(channel.interpolation(), AnimationInterpolation::Linear);
    assert_eq!(
        channel.pre_extrapolation(),
        AnimationExtrapolation::Constant
    );
    assert_eq!(channel.post_extrapolation(), AnimationExtrapolation::Repeat);
    assert_eq!(channel.in_tangents(), None);
    assert_eq!(channel.out_tangents(), None);
}

fn assert_finished(resources: &PostProcessResourceEvidence) {
    assert_eq!(resources.current_live_bytes(), 0);
    assert_eq!(resources.active_reservations(), 0);
    assert!(resources.committed_canonical_bytes() > 0);
}

fn assert_source(scene: &ValidatedScene, fixture: Fixture) {
    assert_structure(scene);
    for (camera, expected) in scene.cameras().zip(cameras(fixture)) {
        assert_camera_bits(camera.projection(), expected);
    }
    for (light, expected) in scene.lights().zip(lights(fixture)) {
        assert_light_bits(light.definition(), expected);
    }
    assert_bits(positions(scene), &POSITIONS);
    assert_eq!(
        scene
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap()
            .indices(),
        Some(&[0, 1, 2][..]),
    );
    assert_eq!(
        scene
            .roots()
            .next()
            .unwrap()
            .authored_transform()
            .authored_trs()
            .unwrap()
            .translation(),
        Vec3::new(1.0, 2.0, 3.0),
    );
    assert_bits(
        scene
            .animation_clips()
            .next()
            .unwrap()
            .channels()
            .next()
            .unwrap()
            .values(),
        &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
    );
}

fn assert_converted(scene: &ValidatedScene) {
    assert_structure(scene);
    let standard = LocalAxes::try_new(Axis::PositiveY, Axis::PositiveZ, Axis::PositiveX).unwrap();
    let unusual = LocalAxes::try_new(Axis::PositiveZ, Axis::PositiveX, Axis::NegativeY).unwrap();
    let reflected = LocalAxes::try_new(Axis::NegativeY, Axis::PositiveZ, Axis::PositiveX).unwrap();
    let expected_cameras = [
        CameraProjection::perspective(
            standard,
            f32::from_bits(0x3f80_0001),
            Some(f32::from_bits(0x3fc0_0001)),
            0.25,
            Some(128.0),
        )
        .unwrap(),
        CameraProjection::orthographic(unusual, 4.0, 6.0, 0.0, 64.0).unwrap(),
    ];
    for (camera, expected) in scene.cameras().zip(expected_cameras) {
        assert_camera_bits(camera.projection(), expected);
    }
    let root = scene.roots().next().unwrap();
    let projection = root.camera().unwrap().projection();
    // T(1,2,3) * I * [2,3,-5] = [3,5,-2]; sQ maps that Scene point to [4,6,10].
    let local_point = affine_point(projection.axes().basis_matrix(), [4.0, 6.0, -10.0]);
    assert_eq!(
        affine_point(root.authored_transform().local_matrix(), local_point),
        [4.0, 6.0, 10.0],
    );
    let original = lights(Fixture::Valid);
    let expected_kinds = [
        LightKind::directional(),
        LightKind::point(Some(8.0)).unwrap(),
        LightKind::spot(Some(32.0), -0.0, f32::from_bits(0x3f40_0001)).unwrap(),
    ];
    for (index, (light, axes)) in scene
        .lights()
        .zip([standard, unusual, reflected])
        .enumerate()
    {
        let definition = light.definition();
        assert_eq!(definition.axes(), axes);
        assert_eq!(definition.kind(), expected_kinds[index]);
        let tint = definition.tint();
        let source = original[index];
        assert_bits(
            &[tint.x, tint.y, tint.z, definition.intensity()],
            &[
                source.tint().x,
                source.tint().y,
                source.tint().z,
                source.intensity(),
            ],
        );
        assert_eq!(
            definition.kind().inner_cone_radians().map(f32::to_bits),
            source.kind().inner_cone_radians().map(f32::to_bits)
        );
        assert_eq!(
            definition.kind().outer_cone_radians().map(f32::to_bits),
            source.kind().outer_cone_radians().map(f32::to_bits)
        );
    }
    assert_bits(
        positions(scene),
        &[-6.0, 2.0, 4.0, -6.0, 4.0, 4.0, -6.0, 2.0, 6.0],
    );
    assert_eq!(
        scene
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap()
            .indices(),
        Some(&[0, 2, 1][..])
    );
    assert_eq!(
        scene
            .roots()
            .next()
            .unwrap()
            .authored_transform()
            .authored_trs()
            .unwrap()
            .translation(),
        Vec3::new(-6.0, 2.0, 4.0)
    );
    assert_bits(
        scene
            .animation_clips()
            .next()
            .unwrap()
            .channels()
            .next()
            .unwrap()
            .values(),
        &[-6.0, 2.0, 4.0, -12.0, 8.0, 10.0],
    );
}

#[test]
fn shared_definitions_convert_frames_and_distances_without_changing_placements_or_photometry() {
    let (input, evidence) = import_fixture(Fixture::Valid).into_parts();
    let generation = input.generation();
    let (scene, report) = convert_scene_space(input, options(0.5), PostProcessControl::default())
        .unwrap()
        .into_parts();
    assert_converted(&scene);
    assert_ne!(scene.generation(), generation);
    assert!(report.matches_scene(&scene));
    assert!(!evidence.matches_scene(&scene));
    assert_eq!(report.disposition(), PostProcessRunDisposition::Changed);
    assert_finished(report.resources());
    let effects = report.space_conversion_effects().unwrap();
    assert_eq!(effects.source(), TargetSpace::STANDARD);
    assert_eq!(effects.target(), options(0.5).target);
    assert!(effects.reverses_triangle_winding());
}

#[test]
fn repeated_inverse_conversions_restore_exact_frames_without_growing_the_hierarchy() {
    let mut scene = import_fixture(Fixture::Valid).into_parts().0;
    for _ in 0..4 {
        let (converted, report) =
            convert_scene_space(scene, options(0.5), PostProcessControl::default())
                .unwrap()
                .into_parts();
        assert_converted(&converted);
        assert_finished(report.resources());
        let (restored, inverse_report) = convert_scene_space(
            converted,
            SpaceConversionOptions::default(),
            PostProcessControl::default(),
        )
        .unwrap()
        .into_parts();
        assert_source(&restored, Fixture::Valid);
        assert_finished(inverse_report.resources());
        scene = restored;
    }
    let generation = scene.generation();
    let (scene, report) = convert_scene_space(
        scene,
        SpaceConversionOptions::default(),
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    assert_eq!(scene.generation(), generation);
    assert_eq!(report.disposition(), PostProcessRunDisposition::Unchanged);
    assert_eq!(report.resources().committed_canonical_bytes(), 0);
    assert_source(&scene, Fixture::Valid);
}

#[test]
fn unrepresentable_planes_extents_and_ranges_return_typed_errors_and_the_original_input() {
    for (fixture, units, expected) in [
        (
            Fixture::PerspectiveOverflow,
            0.5,
            SpaceConversionIssue::NumericCamera {
                camera: 0,
                source: NumericConversionError::NonFiniteOutput,
            },
        ),
        (
            Fixture::PerspectiveUnderflow,
            4.0,
            SpaceConversionIssue::NumericCamera {
                camera: 0,
                source: NumericConversionError::UnderflowToZero,
            },
        ),
        (
            Fixture::OrthographicOverflow,
            0.5,
            SpaceConversionIssue::NumericCamera {
                camera: 1,
                source: NumericConversionError::NonFiniteOutput,
            },
        ),
        (
            Fixture::OrthographicUnderflow,
            4.0,
            SpaceConversionIssue::NumericCamera {
                camera: 1,
                source: NumericConversionError::UnderflowToZero,
            },
        ),
        (
            Fixture::PerspectiveCollapsed,
            2.0,
            SpaceConversionIssue::CameraProjection {
                camera: 0,
                source: CameraProjectionError::FarPlane,
            },
        ),
        (
            Fixture::OrthographicCollapsed,
            2.0,
            SpaceConversionIssue::CameraProjection {
                camera: 1,
                source: CameraProjectionError::FarPlane,
            },
        ),
        (
            Fixture::PointOverflow,
            0.5,
            SpaceConversionIssue::NumericLight {
                light: 1,
                source: NumericConversionError::NonFiniteOutput,
            },
        ),
        (
            Fixture::SpotUnderflow,
            4.0,
            SpaceConversionIssue::NumericLight {
                light: 2,
                source: NumericConversionError::UnderflowToZero,
            },
        ),
    ] {
        let (input, evidence) = import_fixture(fixture).into_parts();
        let generation = input.generation();
        let position_ptr = positions(&input).as_ptr();
        let failure =
            convert_scene_space(input, options(units), PostProcessControl::default()).unwrap_err();
        assert_eq!(failure.input().generation(), generation, "{fixture:?}");
        assert!(evidence.matches_scene(failure.input()), "{fixture:?}");
        assert_source(failure.input(), fixture);
        assert_eq!(positions(failure.input()).as_ptr(), position_ptr);
        let PostProcessRunError::SpaceConversion { failure } = failure.error() else {
            panic!("typed space-conversion failure expected: {fixture:?}");
        };
        assert!(
            matches!(failure.cause(), SpaceConversionCause::Issue { issue } if *issue == expected),
            "{fixture:?}: {:?}",
            failure.cause()
        );
        let resources = failure.resources().unwrap();
        assert_eq!(resources.committed_canonical_bytes(), 0);
        assert_eq!(resources.current_live_bytes(), 0);
        assert_eq!(resources.active_reservations(), 0);
    }
}

#[test]
fn shared_spatial_conversion_and_detached_evidence_fit_a_one_mib_stack() {
    let (scene, evidence) = import_fixture(Fixture::Valid).into_parts();
    let success = std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(move || {
            let success =
                convert_scene_space(scene, options(0.5), PostProcessControl::default()).unwrap();
            assert_converted(success.scene());
            assert_finished(success.report().resources());
            success
        })
        .unwrap()
        .join()
        .expect("camera/light conversion must fit an ordinary caller stack");
    let (scene, report) = success.into_parts();
    assert!(!evidence.matches_scene(&scene));
    let generation = scene.generation();
    drop(scene);
    let effects = report.space_conversion_effects().unwrap();
    assert_eq!(effects.output_generation(), generation);
    assert_finished(report.resources());
}

#[test]
fn host_unwind_releases_the_converted_spatial_owner_and_both_reports() {
    const PAYLOAD: &str = "host unwind while owning converted cameras and lights";
    let run = || {
        let unwound = std::panic::catch_unwind(|| {
            let (input, evidence) = import_fixture(Fixture::Valid).into_parts();
            let (scene, report) =
                convert_scene_space(input, options(0.5), PostProcessControl::default())
                    .unwrap()
                    .into_parts();
            assert_converted(&scene);
            assert_finished(report.resources());
            std::hint::black_box((&scene, &report, &evidence));
            // Bypass the global hook so the probe measures the owners and caught payload.
            std::panic::resume_unwind(Box::new(PAYLOAD));
        });
        let payload = unwound.expect_err("the host must unwind through its published owners");
        assert_eq!(payload.downcast_ref::<&str>(), Some(&PAYLOAD));
        drop(payload);
    };
    run();
    let allocations = allocation_counter::measure(run);
    assert!(allocations.count_total > 0);
    assert!(allocations.bytes_max > 0);
    assert_eq!(allocations.count_current, 0);
    assert_eq!(allocations.bytes_current, 0);
}
