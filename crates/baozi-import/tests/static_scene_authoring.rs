#![allow(
    clippy::result_large_err,
    reason = "authoring regressions preserve complete terminal evidence for their assertions"
)]

use std::error::Error;

use baozi_core::{AttributeData, SceneSnapshot, SceneSnapshotLimits};
use baozi_import::authoring::{
    AlphaMode, AnimationAuthoringBatch, AppearanceAuthoringBatch, AttributeDomain,
    AttributeSemantic, AuthoredTransform, AuthoringRange, ColorInterpretation, ContentDigest,
    DeformationAuthoringBatch, FormatId, GeometryAttributeAuthoring, GeometryAuthoringBatch,
    GeometryLogicalMeshAuthoring, GeometryMeshInstanceAuthoring, GeometryPrimitiveAuthoring,
    GeometryTopologyAuthoring, ImageAuthoring, ImageLocalId, LineageAuthoringBatch,
    LogicalMeshLocalId, LogicalMorphAuthoring, MaterialAuthoring, MaterialBindingAuthoring,
    MaterialFactors, MaterialLocalId, MeshInstanceDeformationAuthoring, MetadataAuthoringBatch,
    NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, NodeParent, ObservedResourceAuthoring,
    OwnerProvenanceAuthoring, Presence, PrimitiveDeformationAuthoring, ProfileAuthoringBatch,
    ProfileOwner, ProvenanceLocalId, ProvenanceNodeAuthoring, ResourceIdentity,
    ResourceIdentityLocalId, SamplerAuthoring, SamplerLocalId, SamplerState, SceneAuthoringBatch,
    SceneSpace, SessionOpaqueReason, ShadingModel, SourceIdentityAuthoring, SourceIdentityLocalId,
    SourceObjectKind, SourceObjectLocator, SpatialAuthoringBatch, StaticSceneBuilder,
    TextureAuthoring, TextureBlendOperation, TextureChannelSwizzle, TextureCoordinateSource,
    TextureLocalId, TextureRole, TextureStrength, TextureTransform,
};
use baozi_import::{
    CapabilityStatus, CauseOwner, DetectionOutcome, DetectionResult, DetectionScope,
    FormatCapability, FormatDescriptor, FormatEncoding, FormatImporter, ImportCause,
    ImportConfigOverlay, ImportDimension, ImportOperationControl, ImportReport, ImportRequest,
    ImportRequestLayer, ImportResult, OptionCompileLimits, ParseResult, ParseScope, ReadConfidence,
    ReadHint, RegistryBuildLimits, RegistryBuilder, ResourceLimitsOverlay, SceneDraft,
    SceneValidationDomain, SceneValidationError, SceneValidationLimit, import_bytes,
};
use baozi_io::AssetPath;
use sha2::{Digest, Sha256};

const FORMAT: &str = "dev.baozi.test.static-scene";
const PATH: &str = "models/triangle.static";
const SOURCE: &[u8] = b"abc";
const IMAGE: &[u8] = b"encoded-image";

#[derive(Clone, Copy)]
enum Input {
    Triangle,
    ExplicitTriangle,
    TwoMeshes,
    Appearance,
    PrimaryImage,
    KnownSpace,
    EmptyPresent,
    EmptyAbsent,
    InvalidIndex,
    InvalidParent,
}

struct ExpectedDocument {
    resource: ResourceIdentity,
    format: FormatId,
    digest: ContentDigest,
    reason: SessionOpaqueReason,
}

struct CustomImporter {
    input: Input,
    expected: Option<ExpectedDocument>,
}

impl FormatImporter for CustomImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(FormatId::new(FORMAT).unwrap(), "Static Scene fixture")
            .extensions(["static"])
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
        if matches!(self.input, Input::EmptyPresent | Input::EmptyAbsent) {
            let roots = if matches!(self.input, Input::EmptyPresent) {
                Presence::Present(&[][..])
            } else {
                Presence::Absent
            };
            return scope.submit_static_scene(StaticSceneBuilder::new(roots, &[]));
        }

        let roots = [NodeLocalId::new(0), NodeLocalId::new(1)];
        let second_parent = if matches!(self.input, Input::InvalidParent) {
            Presence::Present(NodeParent::new(NodeLocalId::new(3), 0))
        } else {
            Presence::Absent
        };
        let nodes = [
            NodeAuthoringNode::new(
                Presence::Absent,
                AuthoredTransform::identity_trs(),
                Presence::Absent,
                Presence::Absent,
            )
            .with_name("triangle"),
            NodeAuthoringNode::new(
                second_parent,
                AuthoredTransform::identity_trs(),
                Presence::Absent,
                Presence::Absent,
            ),
        ];
        let positions = [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let attributes = [
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                &positions,
                Presence::Absent,
            ),
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                &positions,
                Presence::Absent,
            ),
        ];
        let indices = [
            0,
            1,
            if matches!(self.input, Input::InvalidIndex) {
                3
            } else {
                2
            },
        ];
        let has_appearance = matches!(self.input, Input::Appearance | Input::PrimaryImage);
        let material = if has_appearance {
            Presence::Present(MaterialLocalId::new(0))
        } else {
            Presence::Absent
        };
        let primitives = [
            GeometryPrimitiveAuthoring::new(
                GeometryTopologyAuthoring::TriangleList,
                Presence::Present(&indices),
                material,
                AuthoringRange::new(0, 1),
            ),
            GeometryPrimitiveAuthoring::new(
                GeometryTopologyAuthoring::TriangleList,
                Presence::Present(&indices),
                Presence::Absent,
                AuthoringRange::new(1, 1),
            ),
        ];
        let meshes = [
            GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1)),
            GeometryLogicalMeshAuthoring::new(AuthoringRange::new(1, 1)),
        ];
        let instances = [
            GeometryMeshInstanceAuthoring::new(NodeLocalId::new(0), LogicalMeshLocalId::new(0)),
            GeometryMeshInstanceAuthoring::new(NodeLocalId::new(1), LogicalMeshLocalId::new(1)),
        ];
        let row_count = if matches!(self.input, Input::TwoMeshes) {
            2
        } else {
            1
        };
        let node_count = if matches!(self.input, Input::InvalidParent) {
            2
        } else {
            row_count
        };
        let roots = Presence::Present(&roots[..row_count]);
        let nodes = &nodes[..node_count];
        let meshes = &meshes[..row_count];
        let primitives = &primitives[..row_count];
        let attributes = &attributes[..row_count];
        let instances = &instances[..row_count];

        if let Some(expected) = &self.expected {
            return submit_explicit_triangle(
                scope,
                expected,
                NodeAuthoringBatch::new(roots, nodes),
                GeometryAuthoringBatch::new(meshes, primitives, attributes, instances),
            );
        }

        // The custom plugin supplies only its authored tables; empty deformation rows and the
        // complete document profile belong to the session's submission helper.
        let mut builder = StaticSceneBuilder::new(roots, nodes)
            .with_geometry(meshes, primitives, attributes, instances);
        let known_space = SceneSpace::STANDARD_TARGET;
        if matches!(self.input, Input::KnownSpace) {
            builder = builder.with_scene_space(&known_space);
        }
        if !has_appearance {
            return scope.submit_static_scene(builder);
        }

        let images = [if matches!(self.input, Input::PrimaryImage) {
            ImageAuthoring::session_resource(ResourceIdentityLocalId::new(0), Presence::Absent)
        } else {
            ImageAuthoring::encoded(IMAGE, Presence::Absent)
        }];
        let samplers = [SamplerAuthoring::new(SamplerState::CANONICAL_DEFAULT)];
        let textures = [TextureAuthoring::new(
            ImageLocalId::new(0),
            SamplerLocalId::new(0),
        )];
        let role = TextureRole::BaseColor;
        let coordinates = TextureCoordinateSource::ObjectPosition;
        let channels = TextureChannelSwizzle::RGBA;
        let interpretation = ColorInterpretation::SrgbColor;
        let blend = TextureBlendOperation::Replace;
        let bindings = [MaterialBindingAuthoring::try_from_parts(
            TextureLocalId::new(0),
            role,
            coordinates,
            channels,
            interpretation,
            TextureTransform::IDENTITY,
            TextureStrength::ONE,
            blend,
            None,
        )
        .unwrap()];
        let shading = ShadingModel::Unlit;
        let alpha = AlphaMode::Opaque;
        let materials = [MaterialAuthoring::try_from_parts(
            AuthoringRange::new(0, 1),
            shading,
            alpha,
            false,
            MaterialFactors::empty(),
        )
        .unwrap()
        .with_name("paint")];
        scope.submit_static_scene(
            builder.with_appearance(&images, &samplers, &textures, &bindings, &materials),
        )
    }
}

// This deliberately spells out the existing full authoring contract only as a semantic oracle.
// Expected identity allocation and hashing happen in the test harness before the session starts.
fn submit_explicit_triangle(
    scope: &ParseScope<'_>,
    expected: &ExpectedDocument,
    nodes: NodeAuthoringBatch<'_>,
    geometry: GeometryAuthoringBatch<'_>,
) -> ParseResult<SceneDraft> {
    let resources = [ObservedResourceAuthoring::session_opaque(
        &expected.resource,
        expected.reason,
    )];
    let locator = SourceObjectLocator::indexed(SourceObjectKind::Document, 0);
    let sources =
        [
            SourceIdentityAuthoring::new(
                ResourceIdentityLocalId::new(0),
                &expected.format,
                &locator,
            )
            .with_content_digest(expected.digest),
        ];
    let provenance = [ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(
        0,
    ))];
    let ids = [ProvenanceLocalId::new(0)];
    let owners = [
        ProfileOwner::Scene,
        ProfileOwner::SyntheticRoot,
        ProfileOwner::Node(NodeLocalId::new(0)),
        ProfileOwner::LogicalMesh(0),
        ProfileOwner::Primitive(0),
        ProfileOwner::Attribute(0),
        ProfileOwner::MeshInstance(0),
    ]
    .map(|owner| OwnerProvenanceAuthoring::new(owner, &ids));
    let logical_morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
    let primitive_deformations = [PrimitiveDeformationAuthoring::new(Presence::Absent, &[])];
    let instance_deformations = [MeshInstanceDeformationAuthoring::new(
        Presence::Absent,
        Presence::Absent,
    )];
    let space = SceneSpace::UNKNOWN;
    scope.submit_scene(SceneAuthoringBatch::new(
        SpatialAuthoringBatch::new(&space, &[], &[]),
        nodes,
        AppearanceAuthoringBatch::empty(),
        geometry,
        DeformationAuthoringBatch::new(
            &[],
            &[],
            &logical_morphs,
            &primitive_deformations,
            &instance_deformations,
        ),
        AnimationAuthoringBatch::empty(),
        ProfileAuthoringBatch::new(
            LineageAuthoringBatch::new(&resources, &sources, &provenance, &owners),
            MetadataAuthoringBatch::empty(),
        ),
    ))
}

fn import_with(
    input: Input,
    path: &str,
    bytes: &[u8],
    limits: ResourceLimitsOverlay,
) -> ImportResult<ImportReport> {
    let expected = matches!(input, Input::ExplicitTriangle).then(|| {
        let hash = Sha256::digest(bytes);
        let (resource, reason) = match ResourceIdentity::new(path) {
            Ok(resource) => (resource, SessionOpaqueReason::CallerLabel),
            Err(_) => {
                use std::fmt::Write as _;
                let mut identity = String::from("document/sha256/");
                for byte in &hash {
                    write!(identity, "{byte:02x}").unwrap();
                }
                (
                    ResourceIdentity::new(identity).unwrap(),
                    SessionOpaqueReason::SessionFallback,
                )
            }
        };
        ExpectedDocument {
            resource,
            format: FormatId::new(FORMAT).unwrap(),
            digest: ContentDigest::sha256(hash.into()),
            reason,
        }
    });
    let mut builder = RegistryBuilder::new();
    builder
        .register(CustomImporter { input, expected })
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
    import_bytes(
        &request,
        AssetPath::new(path).unwrap(),
        bytes,
        ImportOperationControl::default(),
    )
}

fn import(input: Input) -> ImportReport {
    import_with(input, PATH, SOURCE, ResourceLimitsOverlay::default()).unwrap()
}

fn snapshot(report: &ImportReport) -> SceneSnapshot {
    report
        .scene()
        .capture_snapshot(SceneSnapshotLimits::try_new(64 * 1024, 256, 100_000).unwrap())
        .unwrap()
}

#[test]
fn custom_triangle_needs_only_nodes_and_geometry() {
    let report = import(Input::Triangle);
    let scene = report.scene();
    assert_eq!(scene.mesh_count(), 1);
    assert_eq!(scene.primitive_count(), 1);
    assert_eq!(scene.mesh_instance_count(), 1);
    assert_eq!(scene.material_count(), 0);
    assert_eq!(scene.image_count(), 0);
    assert!(scene.space().handedness.is_unknown());
    assert!(scene.space().unit_scale_to_meters.is_none());
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    assert_eq!(primitive.indices(), Some(&[0, 1, 2][..]));
    let AttributeData::DenseF32 { lanes: 3, values } = primitive.positions().unwrap().data() else {
        panic!("expected borrowed F32 position payload");
    };
    assert_eq!(values, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
    let snapshot = snapshot(&report);
    assert_eq!(snapshot.profile_resource_count(), 1);
    assert_eq!(snapshot.profile_source_count(), 1);
    assert_eq!(snapshot.profile_provenance_node_count(), 1);
    assert_eq!(snapshot.profile_owner_count(), 7);
    assert_eq!(snapshot.profile_attachment_count(), 7);
    assert_eq!(snapshot.profile_metadata_entry_count(), 0);
    assert_eq!(report.ledger().current_live_bytes(), 0);
    assert_eq!(report.ledger().active_reservations(), 0);
}

#[test]
fn generated_empty_deformation_rows_align_with_every_geometry_row() {
    let report = import(Input::TwoMeshes);
    let snapshot = snapshot(&report);
    assert_eq!(snapshot.logical_mesh_count(), 2);
    assert_eq!(snapshot.primitive_count(), 2);
    assert_eq!(snapshot.attribute_count(), 2);
    assert_eq!(snapshot.mesh_instance_count(), 2);
    assert_eq!(snapshot.profile_owner_count(), 12);
    assert_eq!(snapshot.profile_attachment_count(), 12);
}

#[test]
fn appearance_owners_receive_the_same_document_source() {
    let report = import(Input::Appearance);
    let scene = report.scene();
    let material = scene
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap()
        .material()
        .unwrap();
    assert_eq!(material.name(), Some("paint"));
    assert_eq!(
        material
            .bindings()
            .next()
            .unwrap()
            .texture()
            .image()
            .encoded_bytes(),
        IMAGE
    );
    let snapshot = snapshot(&report);
    assert_eq!(snapshot.image_count(), 1);
    assert_eq!(snapshot.sampler_count(), 1);
    assert_eq!(snapshot.texture_count(), 1);
    assert_eq!(snapshot.material_binding_count(), 1);
    assert_eq!(snapshot.material_count(), 1);
    assert_eq!(snapshot.profile_source_count(), 1);
    assert_eq!(snapshot.profile_owner_count(), 12);
    assert_eq!(snapshot.profile_attachment_count(), 12);
}

#[test]
fn primary_document_resource_references_use_the_session_snapshot() {
    let report = import(Input::PrimaryImage);
    assert_eq!(
        report.scene().images().next().unwrap().encoded_bytes(),
        SOURCE
    );
}

#[test]
fn document_profile_matches_explicit_authoring_and_reuses_the_manifest_digest() {
    let automatic = import(Input::Triangle);
    let explicit = import(Input::ExplicitTriangle);
    assert_eq!(snapshot(&automatic).digest(), snapshot(&explicit).digest());
    assert_eq!(
        automatic
            .ledger()
            .dimension(ImportDimension::WorkUnits)
            .unwrap()
            .consumed(),
        explicit
            .ledger()
            .dimension(ImportDimension::WorkUnits)
            .unwrap()
            .consumed(),
        "automatic provenance must reuse the graph digest without rehashing source bytes",
    );
    assert!(
        automatic
            .ledger()
            .dimension(ImportDimension::ScratchBytes)
            .unwrap()
            .peak_live_bytes()
            > explicit
                .ledger()
                .dimension(ImportDimension::ScratchBytes)
                .unwrap()
                .peak_live_bytes(),
        "temporary identity and authoring rows must remain metered during submission",
    );
    let changed = import_with(
        Input::Triangle,
        PATH,
        b"abd",
        ResourceLimitsOverlay::default(),
    )
    .unwrap();
    assert_ne!(snapshot(&automatic).digest(), snapshot(&changed).digest());
}

#[test]
fn unsuitable_document_paths_use_a_digest_identity_with_truthful_fallback_classification() {
    let long_path = format!("models/{}.static", "long-name".repeat(64));
    for path in [long_path.as_str(), "models/triangle#variant.static"] {
        let automatic = import_with(
            Input::Triangle,
            path,
            SOURCE,
            ResourceLimitsOverlay::default(),
        )
        .unwrap();
        let explicit = import_with(
            Input::ExplicitTriangle,
            path,
            SOURCE,
            ResourceLimitsOverlay::default(),
        )
        .unwrap();
        assert_eq!(snapshot(&automatic).digest(), snapshot(&explicit).digest());
    }
}

#[test]
fn known_space_and_absent_root_declarations_are_preserved() {
    let report = import(Input::KnownSpace);
    assert_eq!(report.scene().space().unit_scale_to_meters, Some(1.0));
    let present = import(Input::EmptyPresent);
    assert_eq!(snapshot(&present).profile_owner_count(), 2);
    let absent = import_with(
        Input::EmptyAbsent,
        PATH,
        SOURCE,
        ResourceLimitsOverlay::default(),
    )
    .unwrap_err()
    .into_operation_failure()
    .unwrap();
    assert_eq!(
        absent.scene_validation_error(),
        Some(&SceneValidationError::RootsAbsent)
    );
    assert_eq!(absent.ledger().current_live_bytes(), 0);
    assert_eq!(absent.ledger().active_reservations(), 0);
}

#[test]
fn static_submission_preserves_central_semantic_errors_and_their_source_chain() {
    for (input, expected) in [
        (
            Input::InvalidIndex,
            SceneValidationError::VertexIndexOutOfRange {
                primitive: 0,
                corner: 2,
                index: 3,
                vertex_count: 3,
            },
        ),
        (
            Input::InvalidParent,
            SceneValidationError::ParentOutOfRange {
                node: 1,
                parent: 3,
                node_count: 2,
            },
        ),
    ] {
        let failure = import_with(input, PATH, SOURCE, ResourceLimitsOverlay::default())
            .unwrap_err()
            .into_operation_failure()
            .unwrap();
        assert_eq!(failure.scene_validation_error(), Some(&expected));
        assert!(
            matches!(failure.cause(), ImportCause::SceneValidation(detail) if *detail == expected)
        );
        assert_eq!(failure.cause().owner(), CauseOwner::Core);
        assert!(
            Error::source(failure.cause())
                .unwrap()
                .is::<SceneValidationError>()
        );
        assert_eq!(failure.ledger().current_live_bytes(), 0);
        assert_eq!(failure.ledger().active_reservations(), 0);
    }
}

#[test]
fn static_submission_uses_the_existing_scene_limits() {
    let mut limits = ResourceLimitsOverlay::default();
    limits.max_graph_nodes = Some(2);
    let failure = import_with(Input::TwoMeshes, PATH, SOURCE, limits)
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
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
}
