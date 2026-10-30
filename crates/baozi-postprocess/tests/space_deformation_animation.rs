use baozi_core::{
    AnimationChannelRef, AnimationExtrapolation, AnimationInterpolation, AnimationTarget,
    AnimationTimeDomain, AttributeDomain, Axis, FormatId, FrontFaceWinding, Handedness, Mat4,
    MorphDeltaData, MorphDeltaRef, MorphDeltaSemantic, NumericConversionError,
    SceneAnimationSamplePart, SceneSnapshot, SceneSnapshotLimits, SkinMatrixCorrection,
    SkinMatrixSpace, SkinMatrixSpaces, TargetSpace, ValidatedScene, Vec3,
};
use baozi_import::authoring::{
    AnimationAuthoringBatch, AnimationChannelAuthoring, AnimationClipAuthoring,
    AnimationTargetAuthoring, AppearanceAuthoringBatch, AttributeSemantic, AuthoredTransform,
    AuthoringRange, ContentDigest, DeformationAuthoringBatch, GeometryAttributeAuthoring,
    GeometryAuthoringBatch, GeometryLogicalMeshAuthoring, GeometryMeshInstanceAuthoring,
    GeometryPrimitiveAuthoring, GeometryTopologyAuthoring, InfluenceAuthoring,
    InfluenceRowsAuthoring, LineageAuthoringBatch, LogicalMeshLocalId, LogicalMorphAuthoring,
    MeshInstanceDeformationAuthoring, MetadataAuthoringBatch, MorphDeltaAuthoring,
    MorphDeltaPayloadAuthoring, MorphTargetDefinitionAuthoring, MorphTargetIdentity,
    NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, NodeParent, ObservedResourceAuthoring,
    OwnerProvenanceAuthoring, Presence, PrimitiveDeformationAuthoring,
    PrimitiveMorphTargetAuthoring, ProfileAuthoringBatch, ProfileOwner, ProvenanceLocalId,
    ProvenanceNodeAuthoring, ResourceIdentity, ResourceIdentityLocalId, ResourceSnapshotAuthoring,
    SceneAuthoringBatch, SceneSpace, SessionOpaqueReason, SkinAuthoring, SkinLocalId,
    SourceIdentityAuthoring, SourceIdentityLocalId, SourceObjectKind, SourceObjectLocator,
    SpatialAuthoringBatch, TangentBasisConvention, TangentHandednessEncoding, UvOrigin,
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

const FORMAT: &str = "dev.baozi.test.space-deformation-animation";
const PATH: &str = "fixtures/deformation-animation.spacefixture";
const SOURCE: &[u8] = b"abc";
// SHA-256 of SOURCE, also observed independently by the import manifest.
const SOURCE_DIGEST: [u8; 32] = [
    0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
    0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
];
const INVERSE_BINDS: [Mat4; 2] = [
    Mat4 {
        cols: [
            [2.0, 3.0, 5.0, -0.0],
            [7.0, 11.0, 13.0, 0.0],
            [17.0, 19.0, 23.0, -0.0],
            [29.0, -31.0, 37.0, 1.0],
        ],
    },
    Mat4 {
        cols: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [-2.0, 4.0, 6.0, 1.0],
        ],
    },
];
// The singular linear block prevents a TRS decomposition/inverse from being a valid shortcut.
const CORRECTION: Mat4 = Mat4 {
    cols: [
        [0.0, -0.0, 0.0, -0.0],
        [2.0, 3.0, 4.0, 0.0],
        [-5.0, 6.0, 7.0, -0.0],
        [8.0, -9.0, 10.0, 1.0],
    ],
};
const DENSE_POSITION: [f32; 9] = [1.0, 2.0, 3.0, -4.0, 5.0, -6.0, -0.0, 0.0, 7.0];
const DENSE_NORMAL: [f32; 9] = [2.0, -3.0, 4.0, 5.0, 6.0, -7.0, -0.0, 8.0, 0.0];
const DENSE_TANGENT: [f32; 9] = [-1.0, 2.0, 3.0, 4.0, -5.0, 6.0, 7.0, -0.0, -8.0];
const SPARSE_POSITION: [f32; 6] = [-0.0, 0.0, -0.0, 1.0, -2.0, 3.0];
const SPARSE_NORMAL: [f32; 3] = [4.0, -0.0, 2.0];
const SPARSE_TANGENT: [f32; 6] = [1.0, 2.0, 4.0, -3.0, 0.0, 5.0];
const DEFAULT_WEIGHTS: [f32; 4] = [-0.0, 0.25, 1.0, -0.5];
const OVERRIDE_WEIGHTS: [f32; 4] = [0.75, -0.0, 0.0, 1.0];

const NEXT_ONE: f64 = f64::from_bits(0x3ff0_0000_0000_0001);
const NEXT_TWO: f64 = f64::from_bits(0x4000_0000_0000_0001);
const FRAME_RATE: f64 = f64::from_bits(24.0_f64.to_bits() + 1);
const TICK_RATE: f64 = f64::from_bits(60.0_f64.to_bits() + 1);
const TRANSLATION_TIMES: [f64; 2] = [-2.5, NEXT_TWO];
const ROTATION_TIMES: [f64; 3] = [-0.0, f64::from_bits(1), NEXT_ONE];
const SCALE_TIMES: [f64; 2] = [16_777_217.0, 16_777_217.5];
const WEIGHT_TIMES: [f64; 2] = [-100.25, NEXT_ONE];
const TRANSLATION: [f32; 6] = [1.0, -2.0, 3.0, -0.0, 0.0, -4.0];
const TRANSLATION_IN: [f32; 6] = [0.0, -0.0, 2.0, 3.0, 4.0, -5.0];
const TRANSLATION_OUT: [f32; 6] = [-1.0, 2.0, 0.0, -0.0, -3.0, 4.0];
// The first two keys are q and -q. All three keys are invariant under det(Q)Q;
// only the arbitrary cubic derivatives make this rotation channel a changed owner.
const ROTATION: [f32; 12] = [
    0.5, -0.5, 0.5, 0.5, -0.5, 0.5, -0.5, -0.5, -0.0, 0.0, -0.0, -1.0,
];
const ROTATION_IN: [f32; 12] = [
    2.0, -3.0, 4.0, 5.0, -0.0, 0.0, 0.0, -0.0, -2.0, 0.5, 7.0, -1.0,
];
const ROTATION_OUT: [f32; 12] = [
    0.0, -0.0, 0.0, 0.0, -7.0, 8.0, -9.0, 2.0, 10.0, 11.0, 12.0, -0.0,
];
const SCALE: [f32; 6] = [-2.0, 0.0, 3.0, 4.0, -5.0, -0.0];
const SCALE_IN: [f32; 6] = [-1.0, 2.0, 3.0, 4.0, -5.0, 6.0];
const SCALE_OUT: [f32; 6] = [7.0, 8.0, -9.0, -0.0, 0.0, 10.0];
const WEIGHTS: [f32; 8] = [0.0, -0.0, 0.75, 0.25, 1.0, 0.5, -0.25, -0.0];
const WEIGHTS_IN: [f32; 8] = [2.0, 0.0, -0.0, -1.0, 0.5, -2.0, 3.0, -0.0];
const WEIGHTS_OUT: [f32; 8] = [-0.0, 1.0, 2.0, -3.0, 0.0, 4.0, -5.0, 0.125];
const LATE_TRANSLATION: [f32; 3] = [9.0, -8.0, 7.0];
const LINEAR_ROTATION: [f32; 8] = [-0.0, 0.0, -0.0, 1.0, 0.0, -0.0, 0.0, -1.0];

#[derive(Debug, Clone, Copy)]
enum Fixture {
    Valid,
    CorrectionOverflow,
    InverseBindOverflow,
    DenseMorphOverflow,
    SparseMorphOverflow,
    SparseMorphUnderflow,
    AnimationValueOverflow,
    AnimationTangentOverflow,
}

struct DeformationAnimationImporter {
    fixture: Fixture,
    format: FormatId,
    resource: ResourceIdentity,
}

impl FormatImporter for DeformationAnimationImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            self.format.clone(),
            "Deformation and animation space fixture",
        )
        .encoding(FormatEncoding::Binary)
        .capabilities([
            (FormatCapability::Geometry, CapabilityStatus::Supported),
            (FormatCapability::Animation, CapabilityStatus::Supported),
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
        let mut inverse_binds = INVERSE_BINDS;
        let mut correction = CORRECTION;
        let mut dense_position = DENSE_POSITION;
        let mut sparse_position = SPARSE_POSITION;
        let mut translation_out = TRANSLATION_OUT;
        let mut late_translation = LATE_TRANSLATION;
        match self.fixture {
            Fixture::Valid => {}
            Fixture::CorrectionOverflow => correction.cols[3][2] = f32::MAX,
            Fixture::InverseBindOverflow => inverse_binds[1].cols[3][0] = f32::MAX,
            Fixture::DenseMorphOverflow => dense_position[8] = f32::MAX,
            Fixture::SparseMorphOverflow => sparse_position[3] = f32::MAX,
            Fixture::SparseMorphUnderflow => sparse_position[3] = f32::from_bits(1),
            Fixture::AnimationValueOverflow => late_translation[2] = f32::MAX,
            Fixture::AnimationTangentOverflow => translation_out[5] = f32::MAX,
        }
        let roots = [NodeLocalId::new(0), NodeLocalId::new(1)];
        let nodes = [
            NodeAuthoringNode::new(
                Presence::Absent,
                AuthoredTransform::identity_trs(),
                Presence::Absent,
                Presence::Absent,
            )
            .with_name("mesh"),
            NodeAuthoringNode::new(
                Presence::Absent,
                AuthoredTransform::identity_trs(),
                Presence::Absent,
                Presence::Absent,
            )
            .with_name("rig"),
            NodeAuthoringNode::new(
                Presence::Present(NodeParent::new(NodeLocalId::new(1), 0)),
                AuthoredTransform::identity_trs(),
                Presence::Absent,
                Presence::Absent,
            )
            .with_name("joint"),
        ];
        let attributes = [
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
                Presence::Absent,
            ),
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Normal,
                AttributeDomain::Vertex,
                3,
                &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
                Presence::Absent,
            ),
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Tangent,
                AttributeDomain::Vertex,
                4,
                &[1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0],
                Presence::Absent,
            ),
        ];
        let primitives = [GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Present(&[0, 1, 2]),
            Presence::Absent,
            AuthoringRange::new(0, 3),
        )];
        let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
        let instances = [
            GeometryMeshInstanceAuthoring::new(NodeLocalId::new(0), LogicalMeshLocalId::new(0)),
            GeometryMeshInstanceAuthoring::new(NodeLocalId::new(1), LogicalMeshLocalId::new(0)),
        ];
        let joints = [NodeLocalId::new(2), NodeLocalId::new(1)];
        let skins = [SkinAuthoring::new(
            Presence::Present("Rig α"),
            &joints,
            &inverse_binds,
            Presence::Present(NodeLocalId::new(1)),
            SkinMatrixSpaces::canonical().with_correction(
                SkinMatrixCorrection::new(
                    SkinMatrixSpace::LogicalMeshLocalBind,
                    SkinMatrixSpace::MeshInstanceLocal,
                    correction,
                )
                .unwrap(),
            ),
        )];
        let definitions = [
            MorphTargetDefinitionAuthoring::new(
                MorphTargetIdentity::new(2),
                Presence::Present("dense"),
            ),
            MorphTargetDefinitionAuthoring::new(MorphTargetIdentity::new(0), Presence::Present("")),
            MorphTargetDefinitionAuthoring::new(MorphTargetIdentity::new(3), Presence::Absent),
            MorphTargetDefinitionAuthoring::new(
                MorphTargetIdentity::new(1),
                Presence::Present("explicit-zero"),
            ),
        ];
        let logical_morphs = [LogicalMorphAuthoring::new(
            AuthoringRange::new(0, 4),
            &DEFAULT_WEIGHTS,
        )];
        let dense_deltas = [
            MorphDeltaAuthoring::new(
                MorphDeltaSemantic::Position,
                MorphDeltaPayloadAuthoring::Dense {
                    values: &dense_position,
                },
            ),
            MorphDeltaAuthoring::new(
                MorphDeltaSemantic::Normal,
                MorphDeltaPayloadAuthoring::Dense {
                    values: &DENSE_NORMAL,
                },
            ),
            MorphDeltaAuthoring::new(
                MorphDeltaSemantic::Tangent,
                MorphDeltaPayloadAuthoring::Dense {
                    values: &DENSE_TANGENT,
                },
            ),
        ];
        let sparse_deltas = [
            MorphDeltaAuthoring::new(
                MorphDeltaSemantic::Position,
                MorphDeltaPayloadAuthoring::Sparse {
                    logical_count: 3,
                    indices: &[0, 2],
                    values: &sparse_position,
                },
            ),
            MorphDeltaAuthoring::new(
                MorphDeltaSemantic::Normal,
                MorphDeltaPayloadAuthoring::Sparse {
                    logical_count: 3,
                    indices: &[1],
                    values: &SPARSE_NORMAL,
                },
            ),
            MorphDeltaAuthoring::new(
                MorphDeltaSemantic::Tangent,
                MorphDeltaPayloadAuthoring::Sparse {
                    logical_count: 3,
                    indices: &[0, 2],
                    values: &SPARSE_TANGENT,
                },
            ),
        ];
        let zero_delta = [MorphDeltaAuthoring::new(
            MorphDeltaSemantic::Position,
            MorphDeltaPayloadAuthoring::Sparse {
                logical_count: 3,
                indices: &[],
                values: &[],
            },
        )];
        let targets = [
            PrimitiveMorphTargetAuthoring::new(&dense_deltas),
            PrimitiveMorphTargetAuthoring::new(&sparse_deltas),
            PrimitiveMorphTargetAuthoring::new(&[]),
            PrimitiveMorphTargetAuthoring::new(&zero_delta),
        ];
        let influences = [
            InfluenceAuthoring::new(1, 0.25, 1),
            InfluenceAuthoring::new(0, 0.75, 0),
            InfluenceAuthoring::new(1, 1.0, 0),
        ];
        let primitive_deformations = [PrimitiveDeformationAuthoring::new(
            Presence::Present(InfluenceRowsAuthoring::new(&[0, 2, 2, 3], &influences)),
            &targets,
        )];
        let instance_deformations = [
            MeshInstanceDeformationAuthoring::new(
                Presence::Present(SkinLocalId::new(0)),
                Presence::Present(&OVERRIDE_WEIGHTS),
            ),
            MeshInstanceDeformationAuthoring::new(
                Presence::Present(SkinLocalId::new(0)),
                Presence::Absent,
            ),
        ];
        let channels = [
            AnimationChannelAuthoring::new(
                3,
                AnimationTargetAuthoring::NodeTranslation(NodeLocalId::new(0)),
                AnimationInterpolation::CubicSpline,
                AnimationExtrapolation::Constant,
                AnimationExtrapolation::Linear,
                &TRANSLATION_TIMES,
                &TRANSLATION,
                Presence::Present(&TRANSLATION_IN),
                Presence::Present(&translation_out),
            ),
            AnimationChannelAuthoring::new(
                8,
                AnimationTargetAuthoring::NodeRotation(NodeLocalId::new(2)),
                AnimationInterpolation::CubicSpline,
                AnimationExtrapolation::Repeat,
                AnimationExtrapolation::Constant,
                &ROTATION_TIMES,
                &ROTATION,
                Presence::Present(&ROTATION_IN),
                Presence::Present(&ROTATION_OUT),
            ),
            AnimationChannelAuthoring::new(
                17,
                AnimationTargetAuthoring::NodeScale(NodeLocalId::new(0)),
                AnimationInterpolation::CubicSpline,
                AnimationExtrapolation::Linear,
                AnimationExtrapolation::Repeat,
                &SCALE_TIMES,
                &SCALE,
                Presence::Present(&SCALE_IN),
                Presence::Present(&SCALE_OUT),
            ),
            AnimationChannelAuthoring::new(
                29,
                AnimationTargetAuthoring::MeshInstanceMorphWeights(0),
                AnimationInterpolation::CubicSpline,
                AnimationExtrapolation::SourceDefault,
                AnimationExtrapolation::Repeat,
                &WEIGHT_TIMES,
                &WEIGHTS,
                Presence::Present(&WEIGHTS_IN),
                Presence::Present(&WEIGHTS_OUT),
            ),
            AnimationChannelAuthoring::new(
                45,
                AnimationTargetAuthoring::NodeTranslation(NodeLocalId::new(2)),
                AnimationInterpolation::Step,
                AnimationExtrapolation::Linear,
                AnimationExtrapolation::SourceDefault,
                &[1e100],
                &late_translation,
                Presence::Absent,
                Presence::Absent,
            ),
            AnimationChannelAuthoring::new(
                90,
                AnimationTargetAuthoring::NodeRotation(NodeLocalId::new(1)),
                AnimationInterpolation::Linear,
                AnimationExtrapolation::SourceDefault,
                AnimationExtrapolation::Constant,
                &[-1e100, 1e100],
                &LINEAR_ROTATION,
                Presence::Absent,
                Presence::Absent,
            ),
        ];
        let clips = [
            AnimationClipAuthoring::new(
                7,
                NEXT_ONE,
                AnimationTimeDomain::Seconds,
                AuthoringRange::new(0, 4),
            )
            .with_name("Walk α"),
            AnimationClipAuthoring::new(
                11,
                -0.0,
                AnimationTimeDomain::Frames { rate: FRAME_RATE },
                AuthoringRange::new(4, 2),
            )
            .with_name(""),
            AnimationClipAuthoring::new(
                19,
                0.25,
                AnimationTimeDomain::Ticks {
                    rate: Some(TICK_RATE),
                },
                AuthoringRange::new(6, 0),
            ),
            AnimationClipAuthoring::new(
                25,
                4.0,
                AnimationTimeDomain::Ticks { rate: None },
                AuthoringRange::new(6, 0),
            )
            .with_name("unrated"),
        ];
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
        let attachments = [ProvenanceLocalId::new(0)];
        let mut owners = vec![
            ProfileOwner::Scene,
            ProfileOwner::SyntheticRoot,
            ProfileOwner::Node(NodeLocalId::new(0)),
            ProfileOwner::Node(NodeLocalId::new(1)),
            ProfileOwner::Node(NodeLocalId::new(2)),
            ProfileOwner::LogicalMesh(0),
            ProfileOwner::Primitive(0),
            ProfileOwner::Attribute(0),
            ProfileOwner::Attribute(1),
            ProfileOwner::Attribute(2),
            ProfileOwner::MeshInstance(0),
            ProfileOwner::MeshInstance(1),
            ProfileOwner::Skin(0),
        ];
        owners.extend((0..4).map(ProfileOwner::MorphDefinition));
        owners.extend((0..7).map(ProfileOwner::MorphDelta));
        owners.extend((0..4).map(ProfileOwner::AnimationClip));
        owners.extend((0..6).map(ProfileOwner::AnimationChannel));
        let owners: Vec<_> = owners
            .into_iter()
            .map(|owner| OwnerProvenanceAuthoring::new(owner, &attachments))
            .collect();
        let snapshots = [ResourceSnapshotAuthoring::snapshotted(
            scope.primary_bytes(),
        )];
        let space = SceneSpace {
            uv_origin: UvOrigin::LowerLeft,
            tangent_basis: TangentBasisConvention::MikkTspace,
            tangent_handedness_encoding: TangentHandednessEncoding::SignInW,
            ..SceneSpace::STANDARD_TARGET
        };
        space
            .validate_complete()
            .expect("fixture space must declare complete spatial, UV, and tangent conventions");
        scope.submit_scene(SceneAuthoringBatch::new(
            SpatialAuthoringBatch::new(&space, &[], &[]),
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
        .register(DeformationAnimationImporter {
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
        SOURCE,
        ImportOperationControl::default(),
    )
    .expect("the complete public authoring fixture must pass central import validation")
}

fn mixed_options(units: f64) -> SpaceConversionOptions {
    // Standard source -> Q(x,y,z)=(-z,x,y), det(Q)=-1, s=1/units.
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

fn snapshot(scene: &ValidatedScene) -> SceneSnapshot {
    let snapshot = scene
        .capture_snapshot(SceneSnapshotLimits::try_new(128 * 1024, 256, 1_000_000).unwrap())
        .unwrap();
    assert!(!snapshot.preview_bytes_truncated());
    assert!(!snapshot.preview_items_truncated());
    snapshot
}

fn assert_f32_bits(actual: &[f32], expected: &[f32]) {
    assert_eq!(actual.len(), expected.len());
    for (index, (&actual, &expected)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "F32 scalar {index}: actual {actual:?}, expected {expected:?}"
        );
    }
}

fn assert_matrix_bits(actual: Mat4, expected: Mat4) {
    for (actual, expected) in actual.cols.iter().zip(&expected.cols) {
        assert_f32_bits(actual, expected);
    }
}

fn assert_dense_delta(delta: MorphDeltaRef<'_>, expected: &[f32]) {
    let MorphDeltaData::Dense { values } = delta.data() else {
        panic!("dense morph storage must remain dense");
    };
    assert_eq!(delta.len(), 3);
    assert_f32_bits(values, expected);
}

fn assert_sparse_delta(delta: MorphDeltaRef<'_>, expected_indices: &[u32], expected: &[f32]) {
    let MorphDeltaData::Sparse {
        logical_count,
        indices,
        values,
    } = delta.data()
    else {
        panic!("sparse morph storage must remain sparse");
    };
    assert_eq!(logical_count, 3);
    assert_eq!(indices, expected_indices);
    assert_f32_bits(values, expected);
}

fn assert_deformation(
    scene: &ValidatedScene,
    expected_binds: [Mat4; 2],
    expected_correction: Mat4,
    expected_deltas: [&[f32]; 6],
) {
    assert_eq!(scene.skin_count(), 1);
    let nodes: Vec<_> = scene.nodes().map(|node| node.id()).collect();
    let skin = scene.skins().next().unwrap();
    assert_eq!(skin.name(), Some("Rig α"));
    assert_eq!(skin.skeleton_root().unwrap().id(), nodes[1]);
    assert_eq!(skin.joint_count(), 2);
    assert_eq!(skin.joints().collect::<Vec<_>>(), [nodes[2], nodes[1]]);
    let binds: Vec<_> = skin.inverse_bind_matrices().collect();
    assert_eq!(binds.len(), 2);
    for (actual, expected) in binds.into_iter().zip(expected_binds) {
        assert_matrix_bits(actual, expected);
    }
    let correction = skin.matrix_spaces().correction().unwrap();
    assert_eq!(correction.from(), SkinMatrixSpace::LogicalMeshLocalBind);
    assert_eq!(correction.to(), SkinMatrixSpace::MeshInstanceLocal);
    assert_matrix_bits(correction.matrix(), expected_correction);
    assert_eq!(
        skin.matrix_spaces(),
        SkinMatrixSpaces::canonical().with_correction(correction)
    );

    let mesh = scene.meshes().next().unwrap();
    assert_eq!(mesh.morph_target_count(), 4);
    assert_f32_bits(mesh.default_morph_weights(), &DEFAULT_WEIGHTS);
    let definitions: Vec<_> = mesh.morph_target_definitions().collect();
    for (ordinal, definition) in definitions.iter().copied().enumerate() {
        assert_eq!(definition.ordinal(), ordinal as u32);
        assert_eq!(definition.stable_identity(), [2, 0, 3, 1][ordinal]);
        assert_eq!(
            definition.name(),
            [Some("dense"), Some(""), None, Some("explicit-zero")][ordinal]
        );
        assert_eq!(definition.mesh().id(), mesh.id());
    }
    let primitive = mesh.primitives().next().unwrap();
    let influences = primitive.influences().unwrap();
    assert_eq!(influences.len(), 3);
    let first = influences.get(0).unwrap();
    assert_eq!(first.joint_indices(), [1, 0]);
    assert_f32_bits(first.weights(), &[0.25, 0.75]);
    assert_eq!(first.source_orders(), [1, 0]);
    assert!(influences.get(1).unwrap().is_empty());
    let last = influences.get(2).unwrap();
    assert_eq!(last.joint_indices(), [1]);
    assert_f32_bits(last.weights(), &[1.0]);
    assert_eq!(last.source_orders(), [0]);
    let targets: Vec<_> = primitive.morph_targets().collect();
    assert_eq!(targets.len(), 4);
    for (target, definition) in targets.iter().zip(&definitions) {
        assert_eq!(target.id(), definition.id());
        assert_eq!(
            target.definition().stable_identity(),
            definition.stable_identity()
        );
    }
    let semantics = [
        MorphDeltaSemantic::Position,
        MorphDeltaSemantic::Normal,
        MorphDeltaSemantic::Tangent,
    ];
    for target in &targets[..2] {
        assert_eq!(target.delta_count(), 3);
        assert_eq!(
            target
                .deltas()
                .map(|delta| delta.semantic())
                .collect::<Vec<_>>(),
            semantics
        );
    }
    for (index, semantic) in semantics.into_iter().enumerate() {
        assert_dense_delta(targets[0].delta(semantic).unwrap(), expected_deltas[index]);
        assert_sparse_delta(
            targets[1].delta(semantic).unwrap(),
            if index == 1 { &[1] } else { &[0, 2] },
            expected_deltas[index + 3],
        );
    }
    let sparse_position = targets[1].delta(MorphDeltaSemantic::Position).unwrap();
    assert_eq!(sparse_position.data().value(1), Some(Vec3::ZERO));
    assert_eq!(targets[2].delta_count(), 0);
    for semantic in semantics {
        assert!(targets[2].delta(semantic).is_none());
    }
    assert_eq!(targets[3].delta_count(), 1);
    assert_sparse_delta(
        targets[3].delta(MorphDeltaSemantic::Position).unwrap(),
        &[],
        &[],
    );
    assert!(targets[3].delta(MorphDeltaSemantic::Normal).is_none());
    assert!(targets[3].delta(MorphDeltaSemantic::Tangent).is_none());
    let instances: Vec<_> = scene.mesh_instances().collect();
    assert_eq!(instances.len(), 2);
    assert_f32_bits(instances[0].morph_weights().unwrap(), &OVERRIDE_WEIGHTS);
    assert!(instances[1].morph_weights().is_none());
    for instance in instances {
        assert_eq!(instance.skin().unwrap().id(), skin.id());
        assert_eq!(instance.mesh().id(), mesh.id());
    }
}

fn assert_animation_headers(scene: &ValidatedScene) {
    let clips: Vec<_> = scene.animation_clips().collect();
    assert_eq!(clips.len(), 4);
    for (index, clip) in clips.iter().copied().enumerate() {
        assert_eq!(
            clip.name(),
            [Some("Walk α"), Some(""), None, Some("unrated")][index]
        );
        assert_eq!(
            clip.duration().to_bits(),
            [NEXT_ONE, -0.0, 0.25, 4.0][index].to_bits()
        );
        assert_eq!(clip.channel_count(), [4, 2, 0, 0][index]);
        assert_eq!(
            clip.time_domain(),
            [
                AnimationTimeDomain::Seconds,
                AnimationTimeDomain::Frames { rate: FRAME_RATE },
                AnimationTimeDomain::Ticks {
                    rate: Some(TICK_RATE)
                },
                AnimationTimeDomain::Ticks { rate: None },
            ][index]
        );
    }
    let nodes: Vec<_> = scene.nodes().map(|node| node.id()).collect();
    let instance = scene.mesh_instances().next().unwrap().id();
    let targets = [
        AnimationTarget::NodeTranslation(nodes[0]),
        AnimationTarget::NodeRotation(nodes[2]),
        AnimationTarget::NodeScale(nodes[0]),
        AnimationTarget::MeshInstanceMorphWeights(instance),
        AnimationTarget::NodeTranslation(nodes[2]),
        AnimationTarget::NodeRotation(nodes[1]),
    ];
    let expected_times: [&[f64]; 6] = [
        &TRANSLATION_TIMES,
        &ROTATION_TIMES,
        &SCALE_TIMES,
        &WEIGHT_TIMES,
        &[1e100],
        &[-1e100, 1e100],
    ];
    let extrapolations = [
        (
            AnimationExtrapolation::Constant,
            AnimationExtrapolation::Linear,
        ),
        (
            AnimationExtrapolation::Repeat,
            AnimationExtrapolation::Constant,
        ),
        (
            AnimationExtrapolation::Linear,
            AnimationExtrapolation::Repeat,
        ),
        (
            AnimationExtrapolation::SourceDefault,
            AnimationExtrapolation::Repeat,
        ),
        (
            AnimationExtrapolation::Linear,
            AnimationExtrapolation::SourceDefault,
        ),
        (
            AnimationExtrapolation::SourceDefault,
            AnimationExtrapolation::Constant,
        ),
    ];
    for (index, channel) in clips.iter().flat_map(|clip| clip.channels()).enumerate() {
        assert_eq!(channel.target(), targets[index]);
        assert_eq!(channel.value_width(), [3, 4, 3, 4, 3, 4][index]);
        assert_eq!(channel.key_count(), expected_times[index].len());
        assert_eq!(channel.key_times().len(), expected_times[index].len());
        for (actual, expected) in channel.key_times().iter().zip(expected_times[index]) {
            assert_eq!(actual.to_bits(), expected.to_bits());
        }
        assert_eq!(channel.pre_extrapolation(), extrapolations[index].0);
        assert_eq!(channel.post_extrapolation(), extrapolations[index].1);
        assert_eq!(
            channel.interpolation(),
            match index {
                0..=3 => AnimationInterpolation::CubicSpline,
                4 => AnimationInterpolation::Step,
                _ => AnimationInterpolation::Linear,
            }
        );
    }
}

fn assert_channel(
    channel: AnimationChannelRef<'_>,
    values: &[f32],
    input: Option<&[f32]>,
    output: Option<&[f32]>,
) {
    assert_f32_bits(channel.values(), values);
    for (actual, expected) in [
        (channel.in_tangents(), input),
        (channel.out_tangents(), output),
    ] {
        match (actual, expected) {
            (None, None) => {}
            (Some(actual), Some(expected)) => assert_f32_bits(actual, expected),
            _ => panic!("conversion must preserve cubic tangent presence"),
        }
    }
}

fn assert_source_order(snapshot: &SceneSnapshot) {
    let labels = |prefix| {
        snapshot
            .preview()
            .lines()
            .filter(move |line| line.starts_with(prefix))
            .map(|line| line.split_whitespace().next().unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        labels("  clip["),
        ["clip[7]", "clip[11]", "clip[19]", "clip[25]"]
    );
    assert_eq!(
        labels("  channel["),
        [
            "channel[3]",
            "channel[8]",
            "channel[17]",
            "channel[29]",
            "channel[45]",
            "channel[90]"
        ]
    );
}

fn assert_finished(resources: &PostProcessResourceEvidence) {
    assert_eq!(resources.current_live_bytes(), 0);
    assert_eq!(resources.active_reservations(), 0);
    assert!(resources.consumed_work_items() > 0);
    assert!(resources.attempted_work_items() >= resources.consumed_work_items());
}

#[test]
fn conversion_conjugates_skin_matrices_and_transports_all_morph_representations() {
    let (input, import_evidence) = import_fixture(Fixture::Valid).into_parts();
    let generation = input.generation();
    let (scene, report) = convert_scene_space(
        input,
        mixed_options(0.5),
        PostProcessControl::default(),
    )
    .expect(
        "all four domains must survive materialization, correspondence, and exact work admission",
    )
    .into_parts();
    assert_ne!(scene.generation(), generation);
    assert_eq!(report.disposition(), PostProcessRunDisposition::Changed);
    assert!(report.matches_scene(&scene));
    assert!(!import_evidence.matches_scene(&scene));
    assert_finished(report.resources());
    assert!(report.resources().committed_canonical_bytes() > 0);
    let effects = report.space_conversion_effects().unwrap();
    assert_eq!(effects.source(), TargetSpace::STANDARD);
    assert_eq!(effects.target(), mixed_options(0.5).target);
    assert!(effects.reverses_triangle_winding());
    // H=diag(2Q,1). The linear blocks are Q A Q^T; translations are 2Qt.
    assert_deformation(
        &scene,
        [
            Mat4 {
                cols: [
                    [23.0, -17.0, -19.0, 0.0],
                    [-5.0, 2.0, 3.0, -0.0],
                    [-13.0, 7.0, 11.0, 0.0],
                    [-74.0, 58.0, -62.0, 1.0],
                ],
            },
            Mat4 {
                cols: [
                    [1.0, -0.0, -0.0, -0.0],
                    [-0.0, 1.0, 0.0, 0.0],
                    [-0.0, 0.0, 1.0, 0.0],
                    [-12.0, -4.0, 8.0, 1.0],
                ],
            },
        ],
        Mat4 {
            cols: [
                [7.0, 5.0, -6.0, 0.0],
                [-0.0, 0.0, -0.0, -0.0],
                [-4.0, 2.0, 3.0, 0.0],
                [-20.0, 16.0, -18.0, 1.0],
            ],
        },
        [
            &[-6.0, 2.0, 4.0, 12.0, -8.0, 10.0, -14.0, -0.0, 0.0],
            &[-4.0, 2.0, -3.0, 7.0, 5.0, 6.0, -0.0, -0.0, 8.0],
            &[-3.0, -1.0, 2.0, -6.0, 4.0, -5.0, 8.0, 7.0, -0.0],
            &[0.0, -0.0, 0.0, -6.0, 2.0, -4.0],
            &[-2.0, 4.0, -0.0],
            &[-4.0, 1.0, 2.0, -5.0, -3.0, 0.0],
        ],
    );
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    assert_eq!(primitive.indices().unwrap(), [0, 2, 1]);
    assert_animation_headers(&scene);
    assert_source_order(&snapshot(&scene));
}

#[test]
fn conversion_preserves_curve_metadata_and_maps_arbitrary_cubic_quaternion_derivatives() {
    let (input, _) = import_fixture(Fixture::Valid).into_parts();
    let (scene, report) =
        convert_scene_space(input, mixed_options(0.5), PostProcessControl::default())
            .expect("mixed curve widths and tangent pools must pass exact correspondence work")
            .into_parts();
    assert_finished(report.resources());
    assert_animation_headers(&scene);
    let clips: Vec<_> = scene.animation_clips().collect();
    assert_channel(
        clips[0].channel(0).unwrap(),
        &[-6.0, 2.0, -4.0, 8.0, -0.0, 0.0],
        Some(&[-4.0, 0.0, -0.0, 10.0, 6.0, 8.0]),
        Some(&[-0.0, -2.0, 4.0, -8.0, -0.0, -6.0]),
    );
    // (xyz,w) -> (z,-x,-y,w), including derivatives that are nonunit or all zero.
    let rotation = clips[0].channel(1).unwrap();
    assert_channel(
        rotation,
        &[
            0.5, -0.5, 0.5, 0.5, -0.5, 0.5, -0.5, -0.5, -0.0, 0.0, -0.0, -1.0,
        ],
        Some(&[
            4.0, -2.0, 3.0, 5.0, 0.0, 0.0, -0.0, -0.0, 7.0, 2.0, -0.5, -1.0,
        ]),
        Some(&[
            0.0, -0.0, 0.0, 0.0, -9.0, 7.0, -8.0, 2.0, 12.0, -10.0, -11.0, -0.0,
        ]),
    );
    for (positive, negative) in rotation.values()[..4].iter().zip(&rotation.values()[4..8]) {
        assert_eq!((-*positive).to_bits(), negative.to_bits());
    }
    assert_channel(
        clips[0].channel(2).unwrap(),
        &[3.0, -2.0, 0.0, -0.0, 4.0, -5.0],
        Some(&[3.0, -1.0, 2.0, 6.0, 4.0, -5.0]),
        Some(&[-9.0, 7.0, 8.0, 10.0, -0.0, 0.0]),
    );
    assert_channel(
        clips[0].channel(3).unwrap(),
        &WEIGHTS,
        Some(&WEIGHTS_IN),
        Some(&WEIGHTS_OUT),
    );
    assert_channel(
        clips[1].channel(0).unwrap(),
        &[-14.0, 18.0, -16.0],
        None,
        None,
    );
    assert_channel(clips[1].channel(1).unwrap(), &LINEAR_ROTATION, None, None);
    assert_source_order(&snapshot(&scene));
}

#[test]
fn numeric_failures_retain_typed_dense_locations_and_the_complete_unchanged_input() {
    let cases = [
        (
            Fixture::CorrectionOverflow,
            0.5,
            SpaceConversionIssue::NumericSkinCorrection {
                skin: 0,
                source: NumericConversionError::NonFiniteOutput,
            },
        ),
        (
            Fixture::InverseBindOverflow,
            0.5,
            SpaceConversionIssue::NumericInverseBind {
                matrix: 1,
                source: NumericConversionError::NonFiniteOutput,
            },
        ),
        (
            Fixture::DenseMorphOverflow,
            0.5,
            SpaceConversionIssue::NumericMorphDelta {
                delta: 0,
                tuple: 2,
                source: NumericConversionError::NonFiniteOutput,
            },
        ),
        (
            Fixture::SparseMorphOverflow,
            0.5,
            SpaceConversionIssue::NumericMorphDelta {
                delta: 3,
                tuple: 1,
                source: NumericConversionError::NonFiniteOutput,
            },
        ),
        (
            Fixture::SparseMorphUnderflow,
            2.0,
            SpaceConversionIssue::NumericMorphDelta {
                delta: 3,
                tuple: 1,
                source: NumericConversionError::UnderflowToZero,
            },
        ),
        (
            Fixture::AnimationValueOverflow,
            0.5,
            SpaceConversionIssue::NumericAnimation {
                channel: 4,
                part: SceneAnimationSamplePart::Values,
                key: 0,
                source: NumericConversionError::NonFiniteOutput,
            },
        ),
        (
            Fixture::AnimationTangentOverflow,
            0.5,
            SpaceConversionIssue::NumericAnimation {
                channel: 0,
                part: SceneAnimationSamplePart::OutTangents,
                key: 1,
                source: NumericConversionError::NonFiniteOutput,
            },
        ),
    ];
    for (fixture, units, expected) in cases {
        let (input, import_evidence) = import_fixture(fixture).into_parts();
        let generation = input.generation();
        let before = snapshot(&input);
        let failure =
            convert_scene_space(input, mixed_options(units), PostProcessControl::default())
                .expect_err(
                    "preflight must inspect later matrices, tuples, keys, and tangent streams",
                );
        assert_eq!(failure.input().generation(), generation, "{fixture:?}");
        assert!(
            import_evidence.matches_scene(failure.input()),
            "{fixture:?}"
        );
        let after = snapshot(failure.input());
        assert_eq!(after.digest(), before.digest(), "{fixture:?}");
        assert_eq!(after.preview(), before.preview(), "{fixture:?}");
        let PostProcessRunError::SpaceConversion { failure } = failure.error() else {
            panic!("expected a typed space-conversion failure for {fixture:?}");
        };
        let SpaceConversionCause::Issue { issue } = failure.cause() else {
            panic!(
                "expected numeric issue for {fixture:?}: {:?}",
                failure.cause()
            );
        };
        assert_eq!(*issue, expected, "{fixture:?}");
        let resources = failure.resources().unwrap();
        assert_eq!(resources.committed_canonical_bytes(), 0);
        assert_finished(resources);
    }
}

#[test]
fn winding_only_shares_deformation_and_animation_storage_and_preserves_every_value_bit() {
    let (input, _) = import_fixture(Fixture::Valid).into_parts();
    let source_morph = input
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap()
        .morph_targets()
        .next()
        .unwrap()
        .delta(MorphDeltaSemantic::Position)
        .unwrap();
    let MorphDeltaData::Dense { values } = source_morph.data() else {
        panic!("dense source positions");
    };
    let source_morph_ptr = values.as_ptr();
    let source_animation = input.animation_clips().next().unwrap().channel(1).unwrap();
    let source_values_ptr = source_animation.values().as_ptr();
    let source_times_ptr = source_animation.key_times().as_ptr();
    let source_input_ptr = source_animation.in_tangents().unwrap().as_ptr();
    let source_output_ptr = source_animation.out_tangents().unwrap().as_ptr();
    let options = SpaceConversionOptions {
        target: TargetSpace::try_new(
            Handedness::Right,
            Axis::PositiveY,
            Axis::NegativeZ,
            1.0,
            FrontFaceWinding::Clockwise,
        )
        .unwrap(),
        source_assumptions: None,
    };
    let (scene, report) = convert_scene_space(input, options, PostProcessControl::default())
        .expect("winding-only conversion must admit correspondence over both shared owners")
        .into_parts();
    assert_finished(report.resources());
    assert!(
        report
            .space_conversion_effects()
            .unwrap()
            .reverses_triangle_winding()
    );
    assert_deformation(
        &scene,
        INVERSE_BINDS,
        CORRECTION,
        [
            &DENSE_POSITION,
            &DENSE_NORMAL,
            &DENSE_TANGENT,
            &SPARSE_POSITION,
            &SPARSE_NORMAL,
            &SPARSE_TANGENT,
        ],
    );
    assert_animation_headers(&scene);
    let clips: Vec<_> = scene.animation_clips().collect();
    assert_channel(
        clips[0].channel(0).unwrap(),
        &TRANSLATION,
        Some(&TRANSLATION_IN),
        Some(&TRANSLATION_OUT),
    );
    assert_channel(
        clips[0].channel(1).unwrap(),
        &ROTATION,
        Some(&ROTATION_IN),
        Some(&ROTATION_OUT),
    );
    assert_channel(
        clips[0].channel(2).unwrap(),
        &SCALE,
        Some(&SCALE_IN),
        Some(&SCALE_OUT),
    );
    assert_channel(
        clips[0].channel(3).unwrap(),
        &WEIGHTS,
        Some(&WEIGHTS_IN),
        Some(&WEIGHTS_OUT),
    );
    assert_channel(clips[1].channel(0).unwrap(), &LATE_TRANSLATION, None, None);
    assert_channel(clips[1].channel(1).unwrap(), &LINEAR_ROTATION, None, None);
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    assert_eq!(primitive.indices().unwrap(), [0, 2, 1]);
    let MorphDeltaData::Dense { values } = primitive
        .morph_targets()
        .next()
        .unwrap()
        .delta(MorphDeltaSemantic::Position)
        .unwrap()
        .data()
    else {
        panic!("dense output positions");
    };
    assert_eq!(values.as_ptr(), source_morph_ptr);
    let rotation = clips[0].channel(1).unwrap();
    assert_eq!(rotation.values().as_ptr(), source_values_ptr);
    assert_eq!(rotation.key_times().as_ptr(), source_times_ptr);
    assert_eq!(rotation.in_tangents().unwrap().as_ptr(), source_input_ptr);
    assert_eq!(rotation.out_tangents().unwrap().as_ptr(), source_output_ptr);
    assert_source_order(&snapshot(&scene));
}

#[test]
fn populated_deformation_animation_conversion_fits_a_one_mib_stack() {
    let (input, import_evidence) = import_fixture(Fixture::Valid).into_parts();
    let input_generation = input.generation();
    let success = std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(move || {
            let success =
                convert_scene_space(input, mixed_options(0.5), PostProcessControl::default())
                    .expect("conversion of populated deformation/animation must fit a 1 MiB stack");
            let skin = success.scene().skins().next().unwrap();
            assert_matrix_bits(
                skin.inverse_bind_matrices().next().unwrap(),
                Mat4 {
                    cols: [
                        [23.0, -17.0, -19.0, 0.0],
                        [-5.0, 2.0, 3.0, -0.0],
                        [-13.0, 7.0, 11.0, 0.0],
                        [-74.0, 58.0, -62.0, 1.0],
                    ],
                },
            );
            let rotation = success
                .scene()
                .animation_clips()
                .next()
                .unwrap()
                .channel(1)
                .unwrap();
            assert_f32_bits(
                &rotation.in_tangents().unwrap()[..4],
                &[4.0, -2.0, 3.0, 5.0],
            );
            assert_finished(success.report().resources());
            success
        })
        .unwrap()
        .join()
        .expect("conversion thread must complete on its ordinary caller stack");
    let (scene, report) = success.into_parts();
    assert_animation_headers(&scene);
    assert_eq!(scene.skin_count(), 1);
    assert!(report.matches_scene(&scene));
    assert!(!import_evidence.matches_scene(&scene));
    let output_generation = scene.generation();
    drop(scene);
    drop(import_evidence);
    let effects = report.space_conversion_effects().unwrap();
    assert_eq!(effects.input_generation(), input_generation);
    assert_eq!(effects.output_generation(), output_generation);
    assert_finished(report.resources());
}

#[test]
fn ordinary_host_unwind_releases_populated_converted_scene_and_detached_report() {
    const HOST_UNWIND: &str = "host unwind while owning converted deformation and animation";
    let run = || {
        let unwound = std::panic::catch_unwind(|| {
            let (input, import_evidence) = import_fixture(Fixture::Valid).into_parts();
            let (scene, report) =
                convert_scene_space(input, mixed_options(0.5), PostProcessControl::default())
                    .expect("host unwind must follow a fully published conversion")
                    .into_parts();
            assert_eq!(scene.skin_count(), 1);
            assert_eq!(scene.animation_clip_count(), 4);
            assert!(report.matches_scene(&scene));
            assert_finished(report.resources());
            // Both populated owners, the detached report, and import evidence are live when
            // the caller unwinds. Skip the process-wide panic hook so only their cleanup
            // and the caught payload contribute to the allocation measurement.
            std::hint::black_box((&scene, &report, &import_evidence));
            std::panic::resume_unwind(Box::new(HOST_UNWIND));
        });
        let payload = unwound.expect_err("the caller must unwind through the published owners");
        assert_eq!(payload.downcast_ref::<&str>(), Some(&HOST_UNWIND));
        drop(payload);
    };
    // Initialize thread-local runtime helpers before measuring invocation-owned allocations.
    run();
    let allocations = allocation_counter::measure(run);
    assert!(allocations.count_total > 0);
    assert!(allocations.bytes_max > 0);
    assert_eq!(allocations.count_current, 0);
    assert_eq!(allocations.bytes_current, 0);
}
