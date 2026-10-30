#![allow(
    clippy::result_large_err,
    reason = "submission regressions preserve complete terminal evidence for their assertions"
)]

mod support;

use baozi_core::{EntityNameError, FormatId, SceneSpaceValidationError};
use baozi_import::authoring::{
    AlphaMode, AnimationAuthoringBatch, AnimationChannelAuthoring, AnimationClipAuthoring,
    AnimationExtrapolation, AnimationInterpolation, AnimationTargetAuthoring, AnimationTimeDomain,
    AppearanceAuthoringBatch, AttributeDomain, AttributeSemantic, AuthoredTransform,
    AuthoringRange, CameraLocalId, ColorInterpretation, DeformationAuthoringBatch,
    ExtensionValueRef, GeometryAttributeAuthoring, GeometryAuthoringBatch,
    GeometryLogicalMeshAuthoring, GeometryMeshInstanceAuthoring, GeometryPrimitiveAuthoring,
    GeometryTopologyAuthoring, ImageAuthoring, ImageLocalId, LineageAuthoringBatch,
    LogicalMeshLocalId, LogicalMorphAuthoring, Mat4, MaterialAuthoring, MaterialBindingAuthoring,
    MaterialFactors, MeshInstanceDeformationAuthoring, MetadataAuthoringBatch,
    MetadataAuthoringEntry, NamespacedName, NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId,
    OperationKind, OptionsDigest, OwnerProvenanceAuthoring, Presence,
    PrimitiveDeformationAuthoring, ProfileAuthoringBatch, ProfileOwner, ProvenanceLocalId,
    ProvenanceNodeAuthoring, ProvenanceOperationAuthoring, SamplerAuthoring, SamplerLocalId,
    SamplerState, SceneAuthoringBatch, SceneSpace, ShadingModel, SkinAuthoring, SkinMatrixSpaces,
    SpatialAuthoringBatch, TextureAuthoring, TextureBlendOperation, TextureChannelSwizzle,
    TextureCoordinateSource, TextureLocalId, TextureRole, TextureStrength, TextureTransform,
};
use baozi_import::{
    CapabilityStatus, CauseOwner, DetectionOutcome, DetectionResult, DetectionScope,
    FormatCapability, FormatDescriptor, FormatEncoding, FormatImporter, ImportCause,
    ImportConfigOverlay, ImportFailure, ImportOperationControl, ImportRequest, ImportRequestLayer,
    ImportTerminalCause, ImportTerminalFailureClass, OptionCompileLimits, ParseResult, ParseScope,
    ReadConfidence, ReadHint, RegistryBuildLimits, RegistryBuilder, ResourceLimitsOverlay,
    SceneDraft, SceneValidationDomain, SceneValidationError, SceneValidationLimit, import_bytes,
};
use baozi_io::AssetPath;
use baozi_operation::{CancellationToken, ControlCause};
use std::error::Error;

#[derive(Debug, Clone, Copy)]
enum Input {
    Triangle,
    InvalidParent,
    InvalidVertexIndex,
    InvalidAttributeCardinality,
    InvalidTextureImage,
    InvalidMaterialName,
    InvalidBindingAuthority,
    InvalidSkinJoint,
    InvalidInverseBindCount,
    InvalidAnimationTarget,
    InvalidAnimationValues,
    InvalidAnimationName,
    InvalidAnimationTime,
    InvalidCameraReference,
    InvalidSceneSpace,
    InvalidProfileOwner,
    InvalidMetadataValue,
}

struct CustomImporter {
    input: Input,
    cancel_after_rejection: Option<CancellationToken>,
}

impl FormatImporter for CustomImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("dev.baozi.test.submission-errors").unwrap(),
            "Submission validation fixture",
        )
        .extensions(["submission"])
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
        .build()
        .unwrap()
    }

    fn detect(
        &self,
        _probe: &baozi_import::Probe<'_>,
        _hint: &ReadHint,
        _scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        let result = match self.input {
            Input::InvalidParent => support::submit_invalid_parent_scene(scope),
            _ => submit_triangle(scope, self.input),
        };
        if result.is_err()
            && let Some(cancellation) = &self.cancel_after_rejection
        {
            cancellation.cancel();
        }
        result
    }
}

fn submit_triangle(scope: &ParseScope<'_>, input: Input) -> ParseResult<SceneDraft> {
    let roots = [NodeLocalId::new(0)];
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
        if matches!(input, Input::InvalidCameraReference) {
            Presence::Present(CameraLocalId::new(2))
        } else {
            Presence::Absent
        },
        Presence::Absent,
    )];
    let images = [ImageAuthoring::encoded(b"image", Presence::Absent)];
    let samplers = [SamplerAuthoring::new(SamplerState::CANONICAL_DEFAULT)];
    let textures = [TextureAuthoring::new(
        ImageLocalId::new(if matches!(input, Input::InvalidTextureImage) {
            2
        } else {
            0
        }),
        SamplerLocalId::new(0),
    )];
    let role = TextureRole::BaseColor;
    let coordinates = TextureCoordinateSource::UvSet(0);
    let channel_mapping = TextureChannelSwizzle::RGBA;
    let interpretation = ColorInterpretation::SrgbColor;
    let blend = TextureBlendOperation::Replace;
    let binding = MaterialBindingAuthoring::try_from_parts(
        TextureLocalId::new(0),
        role,
        coordinates,
        channel_mapping,
        interpretation,
        TextureTransform::IDENTITY,
        TextureStrength::ONE,
        blend,
        None,
    )
    .unwrap();
    let bindings = [binding, binding];
    let shading = ShadingModel::Unspecified;
    let alpha = AlphaMode::Opaque;
    let materials = [MaterialAuthoring::try_from_parts(
        AuthoringRange::new(
            0,
            if matches!(input, Input::InvalidBindingAuthority) {
                2
            } else {
                0
            },
        ),
        shading,
        alpha,
        false,
        MaterialFactors::default(),
    )
    .unwrap()
    .with_name(if matches!(input, Input::InvalidMaterialName) {
        "bad\nname"
    } else {
        "material"
    })];
    let appearance = match input {
        Input::InvalidTextureImage => {
            AppearanceAuthoringBatch::new(&images, &samplers, &textures, &[], &[])
        }
        Input::InvalidMaterialName => AppearanceAuthoringBatch::new(&[], &[], &[], &[], &materials),
        Input::InvalidBindingAuthority => {
            AppearanceAuthoringBatch::new(&images, &samplers, &textures, &bindings, &materials)
        }
        _ => AppearanceAuthoringBatch::empty(),
    };
    let positions = [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let normals = [0.0_f32, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
    let normal_values = if matches!(input, Input::InvalidAttributeCardinality) {
        &normals[..6]
    } else {
        &normals[..]
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
            normal_values,
            Presence::Absent,
        ),
    ];
    let indices = [
        0,
        1,
        if matches!(input, Input::InvalidVertexIndex) {
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
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let mesh_instances = [GeometryMeshInstanceAuthoring::new(
        NodeLocalId::new(0),
        LogicalMeshLocalId::new(0),
    )];
    let logical_morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
    let primitive_deformations = [PrimitiveDeformationAuthoring::new(Presence::Absent, &[])];
    let instance_deformations = [MeshInstanceDeformationAuthoring::new(
        Presence::Absent,
        Presence::Absent,
    )];
    let joints = [NodeLocalId::new(
        if matches!(input, Input::InvalidSkinJoint) {
            2
        } else {
            0
        },
    )];
    let inverse_binds = [Mat4::IDENTITY];
    let skins = [SkinAuthoring::new(
        Presence::Absent,
        &joints,
        if matches!(input, Input::InvalidInverseBindCount) {
            &[]
        } else {
            &inverse_binds
        },
        Presence::Absent,
        SkinMatrixSpaces::canonical(),
    )];
    let skin_rows = if matches!(
        input,
        Input::InvalidSkinJoint | Input::InvalidInverseBindCount
    ) {
        &skins[..]
    } else {
        &[]
    };
    let times = [0.0, 1.0];
    let repeated_times = [0.0, 0.0];
    let samples = [0.0_f32; 6];
    let first_channel = AnimationChannelAuthoring::new(
        7,
        AnimationTargetAuthoring::NodeTranslation(NodeLocalId::new(
            if matches!(input, Input::InvalidAnimationTarget) {
                2
            } else {
                0
            },
        )),
        AnimationInterpolation::Linear,
        AnimationExtrapolation::Constant,
        AnimationExtrapolation::Constant,
        &times,
        if matches!(input, Input::InvalidAnimationValues) {
            &samples[..3]
        } else {
            &samples
        },
        Presence::Absent,
        Presence::Absent,
    );
    let channels = [
        first_channel,
        AnimationChannelAuthoring::new(
            11,
            AnimationTargetAuthoring::NodeTranslation(NodeLocalId::new(0)),
            AnimationInterpolation::Linear,
            AnimationExtrapolation::Constant,
            AnimationExtrapolation::Constant,
            &times,
            &samples,
            Presence::Absent,
            Presence::Absent,
        ),
        AnimationChannelAuthoring::new(
            12,
            AnimationTargetAuthoring::NodeScale(NodeLocalId::new(0)),
            AnimationInterpolation::Linear,
            AnimationExtrapolation::Constant,
            AnimationExtrapolation::Constant,
            &repeated_times,
            &samples,
            Presence::Absent,
            Presence::Absent,
        ),
    ];
    let clips = [
        AnimationClipAuthoring::new(
            5,
            1.0,
            AnimationTimeDomain::Seconds,
            AuthoringRange::new(0, 1),
        )
        .with_name(if matches!(input, Input::InvalidAnimationName) {
            "bad\nclip"
        } else {
            "clip"
        }),
        AnimationClipAuthoring::new(
            9,
            1.0,
            AnimationTimeDomain::Seconds,
            AuthoringRange::new(1, 2),
        ),
    ];
    let animation = if matches!(
        input,
        Input::InvalidAnimationTarget
            | Input::InvalidAnimationValues
            | Input::InvalidAnimationName
            | Input::InvalidAnimationTime
    ) {
        if matches!(input, Input::InvalidAnimationTime) {
            AnimationAuthoringBatch::new(&clips, &channels)
        } else {
            AnimationAuthoringBatch::new(&clips[..1], &channels[..1])
        }
    } else {
        AnimationAuthoringBatch::empty()
    };
    let provenance_ids = [ProvenanceLocalId::new(0)];
    let operation_kind = OperationKind::Generated;
    let provenance_nodes = [ProvenanceNodeAuthoring::operation(
        ProvenanceOperationAuthoring::new(
            &operation_kind,
            OptionsDigest::sha256([0; 32]),
            &[],
            &[],
        ),
    )];
    let owner_provenance = [
        OwnerProvenanceAuthoring::new(ProfileOwner::Scene, &provenance_ids),
        OwnerProvenanceAuthoring::new(ProfileOwner::SyntheticRoot, &provenance_ids),
        OwnerProvenanceAuthoring::new(
            ProfileOwner::Node(NodeLocalId::new(
                if matches!(input, Input::InvalidProfileOwner) {
                    2
                } else {
                    0
                },
            )),
            &provenance_ids,
        ),
        OwnerProvenanceAuthoring::new(ProfileOwner::LogicalMesh(0), &provenance_ids),
        OwnerProvenanceAuthoring::new(ProfileOwner::Primitive(0), &provenance_ids),
        OwnerProvenanceAuthoring::new(ProfileOwner::Attribute(0), &provenance_ids),
        OwnerProvenanceAuthoring::new(ProfileOwner::Attribute(1), &provenance_ids),
        OwnerProvenanceAuthoring::new(ProfileOwner::MeshInstance(0), &provenance_ids),
    ];
    let metadata_key = NamespacedName::new("fixture:value").unwrap();
    let metadata = [MetadataAuthoringEntry::new(
        ProfileOwner::Scene,
        &metadata_key,
        ExtensionValueRef::F64(f64::NAN),
    )];
    let mut scene_space = SceneSpace::UNKNOWN;
    if matches!(input, Input::InvalidSceneSpace) {
        scene_space.unit_scale_to_meters = Some(0.0);
    }
    scope.submit_scene(SceneAuthoringBatch::new(
        SpatialAuthoringBatch::new(&scene_space, &[], &[]),
        NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
        appearance,
        GeometryAuthoringBatch::new(&logical_meshes, &primitives, &attributes, &mesh_instances),
        DeformationAuthoringBatch::new(
            skin_rows,
            &[],
            &logical_morphs,
            &primitive_deformations,
            &instance_deformations,
        ),
        animation,
        ProfileAuthoringBatch::new(
            LineageAuthoringBatch::new(&[], &[], &provenance_nodes, &owner_provenance),
            if matches!(input, Input::InvalidMetadataValue) {
                MetadataAuthoringBatch::new(&metadata)
            } else {
                MetadataAuthoringBatch::empty()
            },
        ),
    ))
}

fn import(input: Input) -> baozi_import::ImportResult<baozi_import::ImportReport> {
    import_with(input, None, ResourceLimitsOverlay::default())
}

fn import_with(
    input: Input,
    cancellation: Option<CancellationToken>,
    limits: ResourceLimitsOverlay,
) -> baozi_import::ImportResult<baozi_import::ImportReport> {
    let mut builder = RegistryBuilder::new();
    builder
        .register(CustomImporter {
            input,
            cancel_after_rejection: cancellation.clone(),
        })
        .unwrap();
    let registry = builder.freeze(RegistryBuildLimits::default()).unwrap();
    let request = registry
        .compile_request(
            &ImportRequest::from_layers(
                ImportRequestLayer::default(),
                ImportRequestLayer::new(
                    ImportConfigOverlay::default().with_resource_limits(limits),
                ),
            ),
            OptionCompileLimits::default(),
        )
        .unwrap();
    let control = match cancellation {
        Some(cancellation) => ImportOperationControl::default().with_cancellation(cancellation),
        None => ImportOperationControl::default(),
    };
    import_bytes(
        &request,
        AssetPath::new("asset.submission").unwrap(),
        b"input",
        control,
    )
}

fn failure(input: Input) -> ImportFailure {
    import(input).unwrap_err().into_operation_failure().unwrap()
}

fn assert_requested_failure(failure: &ImportFailure) {
    let ImportTerminalCause::Failure(terminal) = failure.ledger().terminal_cause() else {
        panic!("invalid producer input cannot publish success");
    };
    assert_eq!(terminal.class(), ImportTerminalFailureClass::Requested);
    assert_eq!(failure.cause().owner(), CauseOwner::Core);
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
    assert!(failure.terminal_control_cause().is_none());
}

#[test]
fn valid_custom_triangle_is_accepted() {
    import(Input::Triangle).expect("the same fixture with valid source data must publish");
}

#[test]
fn custom_parent_rejection_retains_producer_local_node_and_parent() {
    let failure = failure(Input::InvalidParent);
    let detail = SceneValidationError::ParentOutOfRange {
        node: 1,
        parent: 2,
        node_count: 2,
    };
    assert!(matches!(failure.cause(), ImportCause::SceneValidation(error) if *error == detail));
    assert_eq!(failure.scene_validation_error(), Some(&detail));
    assert_eq!(detail.domain(), SceneValidationDomain::Node);
    assert_eq!(detail.code(), "node.parent-out-of-range");
    assert!(failure.to_string().contains("node 1"));
    assert!(failure.to_string().contains("parent reference 2"));
    assert!(
        Error::source(failure.cause())
            .unwrap()
            .is::<SceneValidationError>()
    );
    assert_requested_failure(&failure);
}

#[test]
fn custom_index_rejection_retains_primitive_corner_and_needed_vertex_count() {
    let failure = failure(Input::InvalidVertexIndex);
    let detail = SceneValidationError::VertexIndexOutOfRange {
        primitive: 0,
        corner: 2,
        index: 3,
        vertex_count: 3,
    };
    assert_eq!(failure.scene_validation_error(), Some(&detail));
    assert_eq!(detail.domain(), SceneValidationDomain::Geometry);
    assert_eq!(detail.code(), "geometry.vertex-index-out-of-range");
    assert!(failure.to_string().contains("primitive 0"));
    assert!(failure.to_string().contains("corner 2"));
    assert_requested_failure(&failure);
}

#[test]
fn custom_cardinality_rejection_retains_attribute_domain_expected_and_actual() {
    let failure = failure(Input::InvalidAttributeCardinality);
    let detail = SceneValidationError::AttributeCardinality {
        attribute: 1,
        domain: AttributeDomain::Vertex,
        expected: 3,
        actual: 2,
    };
    assert_eq!(failure.scene_validation_error(), Some(&detail));
    assert_eq!(detail.code(), "geometry.attribute-cardinality");
    assert!(failure.to_string().contains("attribute 1"));
    assert!(failure.to_string().contains("requires 3"));
    assert_requested_failure(&failure);
}

#[test]
fn custom_semantic_rejections_retain_domain_rule_and_original_values() {
    let cases = [
        (
            Input::InvalidTextureImage,
            SceneValidationError::TextureImageOutOfRange {
                texture: 0,
                image: 2,
                image_count: 1,
            },
            SceneValidationDomain::Appearance,
            "appearance.texture-image-out-of-range",
        ),
        (
            Input::InvalidMaterialName,
            SceneValidationError::InvalidMaterialName {
                material: 0,
                source: EntityNameError::ControlCharacter { offset: 3 },
            },
            SceneValidationDomain::Appearance,
            "appearance.invalid-material-name",
        ),
        (
            Input::InvalidSkinJoint,
            SceneValidationError::JointNodeOutOfRange {
                skin: 0,
                joint: 0,
                node: 2,
                node_count: 1,
            },
            SceneValidationDomain::Deformation,
            "deformation.joint-node-out-of-range",
        ),
        (
            Input::InvalidInverseBindCount,
            SceneValidationError::InverseBindCountMismatch {
                skin: 0,
                joint_count: 1,
                inverse_bind_count: 0,
            },
            SceneValidationDomain::Deformation,
            "deformation.inverse-bind-count-mismatch",
        ),
        (
            Input::InvalidAnimationTarget,
            SceneValidationError::NodeTargetOutOfRange {
                clip: 0,
                channel: 0,
                node: 2,
                node_count: 1,
            },
            SceneValidationDomain::Animation,
            "animation.node-target-out-of-range",
        ),
        (
            Input::InvalidAnimationValues,
            SceneValidationError::ValueShape {
                clip: 0,
                channel: 0,
                expected: 6,
                actual: 3,
            },
            SceneValidationDomain::Animation,
            "animation.value-shape",
        ),
        (
            Input::InvalidAnimationName,
            SceneValidationError::InvalidClipName {
                clip: 0,
                source: EntityNameError::ControlCharacter { offset: 3 },
            },
            SceneValidationDomain::Animation,
            "animation.invalid-clip-name",
        ),
        (
            Input::InvalidAnimationTime,
            SceneValidationError::TimesNotStrictlyIncreasing {
                clip: 1,
                channel: 1,
                previous_key: 0,
                key: 1,
            },
            SceneValidationDomain::Animation,
            "animation.times-not-strictly-increasing",
        ),
        (
            Input::InvalidCameraReference,
            SceneValidationError::CameraOutOfRange {
                node: 0,
                camera: 2,
                camera_count: 0,
            },
            SceneValidationDomain::Spatial,
            "spatial.camera-out-of-range",
        ),
        (
            Input::InvalidSceneSpace,
            SceneValidationError::InvalidSceneSpace(
                SceneSpaceValidationError::NonPositiveUnitScale,
            ),
            SceneValidationDomain::Spatial,
            "spatial.invalid-scene-space",
        ),
        (
            Input::InvalidProfileOwner,
            SceneValidationError::OwnerOutOfRange {
                owner: ProfileOwner::Node(NodeLocalId::new(2)),
            },
            SceneValidationDomain::Profile,
            "profile.owner-out-of-range",
        ),
        (
            Input::InvalidMetadataValue,
            SceneValidationError::MetadataNonFinite { entry: 0 },
            SceneValidationDomain::Profile,
            "profile.metadata-non-finite",
        ),
    ];
    for (input, expected, domain, code) in cases {
        let failure = failure(input);
        assert_eq!(
            failure.scene_validation_error(),
            Some(&expected),
            "{input:?}"
        );
        assert_eq!(expected.domain(), domain);
        assert_eq!(expected.code(), code);
        assert!(
            matches!(failure.cause(), ImportCause::SceneValidation(error) if *error == expected)
        );
        assert_requested_failure(&failure);
    }
}

#[test]
fn custom_material_conflict_retains_the_nested_authority_rule() {
    let failure = failure(Input::InvalidBindingAuthority);
    let Some(SceneValidationError::InvalidMaterialBindingSemantics { material, source }) =
        failure.scene_validation_error()
    else {
        panic!("the material conflict must retain its semantic source: {failure}");
    };
    assert_eq!(*material, 0);
    assert_eq!(
        source.to_string(),
        "texture binding at index 1 duplicates authority at 0"
    );
    let detail = failure.scene_validation_error().unwrap();
    assert_eq!(detail.domain(), SceneValidationDomain::Appearance);
    assert_eq!(
        detail.code(),
        "appearance.invalid-material-binding-semantics"
    );
    assert!(Error::source(detail).is_some());
    assert!(failure.to_string().contains("duplicates authority at 0"));
    assert_requested_failure(&failure);
}

#[test]
fn initial_admission_retains_the_limit_actual_and_maximum() {
    let mut limits = ResourceLimitsOverlay::default();
    limits.max_graph_nodes = Some(2);
    let failure = import_with(Input::InvalidParent, None, limits)
        .unwrap_err()
        .into_operation_failure()
        .unwrap();
    assert_eq!(
        failure.scene_validation_error(),
        Some(&SceneValidationError::LimitExceeded {
            domain: SceneValidationDomain::Scene,
            limit: SceneValidationLimit::Nodes,
            actual: 3,
            maximum: 2,
        })
    );
}

#[test]
fn terminal_cancellation_preserves_the_primary_semantic_rule() {
    let failure = import_with(
        Input::InvalidVertexIndex,
        Some(CancellationToken::new()),
        ResourceLimitsOverlay::default(),
    )
    .unwrap_err()
    .into_operation_failure()
    .unwrap();
    assert_eq!(
        failure.scene_validation_error(),
        Some(&SceneValidationError::VertexIndexOutOfRange {
            primitive: 0,
            corner: 2,
            index: 3,
            vertex_count: 3,
        })
    );
    assert!(matches!(
        failure.terminal_control_cause(),
        Some(ControlCause::Cancelled { .. })
    ));
    assert_eq!(failure.ledger().current_live_bytes(), 0);
}
