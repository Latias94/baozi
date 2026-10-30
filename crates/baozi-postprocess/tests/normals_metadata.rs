#![allow(
    clippy::result_large_err,
    reason = "public-operation regressions retain the original Scene and complete failure evidence"
)]

use baozi_core::{
    AttributeData, AttributeDomain, AttributeSemanticRef, PrimitiveId, SceneSnapshot,
    SceneSnapshotLimits, ValidatedScene,
};
use baozi_import::authoring::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, AttributeSemantic, AuthoredTransform,
    AuthoringRange, ContentDigest, CustomAttributeSemantic, DeformationAuthoringBatch,
    ExtensionMediaType, ExtensionValueRef, FormatId, GeometryAttributeAuthoring,
    GeometryAuthoringBatch, GeometryLogicalMeshAuthoring, GeometryMeshInstanceAuthoring,
    GeometryPrimitiveAuthoring, GeometryTopologyAuthoring, LineageAuthoringBatch,
    LogicalMeshLocalId, LogicalMorphAuthoring, MeshInstanceDeformationAuthoring,
    MetadataAuthoringBatch, MetadataAuthoringEntry, NamespacedName, NodeAuthoringBatch,
    NodeAuthoringNode, NodeLocalId, ObservedResourceAuthoring, OperationKind, OptionsDigest,
    OwnerProvenanceAuthoring, Presence, PrimitiveDeformationAuthoring, ProfileAuthoringBatch,
    ProfileOwner, ProvenanceLocalId, ProvenanceNodeAuthoring, ProvenanceOperationAuthoring,
    ResourceIdentity, ResourceIdentityLocalId, ResourceSnapshotAuthoring, SceneAuthoringBatch,
    SceneSpace, SessionOpaqueReason, SourceIdentityAuthoring, SourceIdentityLocalId,
    SourceObjectKind, SourceObjectLocator, SpatialAuthoringBatch,
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
    ExistingNormals, NormalCause, NormalMode, NormalOptions, PostProcessControl,
    PostProcessOperation, PostProcessReport, PostProcessResourceEvidence,
    PostProcessRunDisposition, PostProcessRunError, TangentHandling, generate_normals,
};

const FORMAT: &str = "dev.baozi.test.normals-metadata";
const PATH: &str = "fixtures/two-primitives.nmfixture";
const SOURCE: &[u8] = b"abc";
// SHA-256 of the fixed primary bytes. This is the same digest the import manifest observes.
const SOURCE_DIGEST: [u8; 32] = [
    0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
    0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
];
const POSITIONS: [f32; 12] = [0.0, -0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0];
const NORMALS: [f32; 12] = [0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0];
const TANGENTS: [f32; 16] = [
    1.0, -0.0, 0.0, 1.0, 1.0, 0.0, 0.0, -1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0,
];
const TEXCOORDS: [f32; 12] = [0.0, -0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0];
const INDICES: [u32; 6] = [0, 1, 2, 0, 2, 3];
const PRECISE: f64 = f64::from_bits(0x3ff0_0000_0000_0001);
const SECOND_PRECISE: f64 = f64::from_bits(0x4000_0000_0000_0001);
const CUSTOM: [f64; 4] = [1.0, -0.0, PRECISE, 4.0];
const SECOND_POSITIONS: [f32; 9] = [0.0, 0.0, 2.0, 1.0, -0.0, 2.0, 0.0, 1.0, 2.0];
const SECOND_CUSTOM: [f64; 3] = [-0.0, SECOND_PRECISE, -7.0];

struct MetadataImporter {
    authored_normals: bool,
    resource: ResourceIdentity,
    format: FormatId,
}

impl FormatImporter for MetadataImporter {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(self.format.clone(), "Normal metadata fixture")
            .extensions(["nmfixture"])
            .encoding(FormatEncoding::Binary)
            .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
            .build()
            .expect("fixture format descriptor")
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
        assert_eq!(scope.source().as_str(), PATH);
        assert_eq!(scope.primary_bytes(), SOURCE);
        let roots = [NodeLocalId::new(0)];
        let nodes = [NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Absent,
            Presence::Absent,
        )];
        let mut attributes = vec![GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &POSITIONS,
            Presence::Absent,
        )];
        if self.authored_normals {
            attributes.push(GeometryAttributeAuthoring::f32(
                AttributeSemantic::Normal,
                AttributeDomain::Vertex,
                3,
                &NORMALS,
                Presence::Absent,
            ));
        }
        let tangent = attributes.len() as u32;
        attributes.push(GeometryAttributeAuthoring::f32(
            AttributeSemantic::Tangent,
            AttributeDomain::Vertex,
            4,
            &TANGENTS,
            Presence::Absent,
        ));
        let uv = attributes.len() as u32;
        attributes.push(GeometryAttributeAuthoring::f32(
            AttributeSemantic::Texcoord(0),
            AttributeDomain::Corner,
            2,
            &TEXCOORDS,
            Presence::Absent,
        ));
        let groups = [7, 7];
        attributes.push(GeometryAttributeAuthoring::u32(
            AttributeSemantic::SmoothingGroup,
            AttributeDomain::Face,
            1,
            &groups,
        ));
        let custom = attributes.len() as u32;
        let semantic = CustomAttributeSemantic::new("test:precision").expect("custom semantic");
        attributes.push(GeometryAttributeAuthoring::f64(
            AttributeSemantic::Custom(semantic.clone()),
            AttributeDomain::Vertex,
            1,
            &CUSTOM,
        ));
        let second_start = attributes.len() as u32;
        attributes.push(GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &SECOND_POSITIONS,
            Presence::Absent,
        ));
        attributes.push(GeometryAttributeAuthoring::f64(
            AttributeSemantic::Custom(semantic),
            AttributeDomain::Vertex,
            1,
            &SECOND_CUSTOM,
        ));
        let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 2))];
        let primitives = [
            GeometryPrimitiveAuthoring::new(
                GeometryTopologyAuthoring::TriangleList,
                Presence::Present(&INDICES),
                Presence::Absent,
                AuthoringRange::new(0, second_start),
            ),
            GeometryPrimitiveAuthoring::new(
                GeometryTopologyAuthoring::TriangleList,
                Presence::Absent,
                Presence::Absent,
                AuthoringRange::new(second_start, 2),
            ),
        ];
        let instances = [GeometryMeshInstanceAuthoring::new(
            NodeLocalId::new(0),
            LogicalMeshLocalId::new(0),
        )];
        let logical_morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
        let primitive_deformations = [PrimitiveDeformationAuthoring::new(Presence::Absent, &[]); 2];
        let instance_deformations = [MeshInstanceDeformationAuthoring::new(
            Presence::Absent,
            Presence::Absent,
        )];

        // Explicit source identity matches the selected format, primary path, stability reason,
        // and observed content digest. Generated provenance remains rooted in that source.
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
        let generated_kind = OperationKind::Generated;
        let parents = [ProvenanceLocalId::new(0)];
        let provenance = [
            ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(0)),
            ProvenanceNodeAuthoring::operation(ProvenanceOperationAuthoring::new(
                &generated_kind,
                OptionsDigest::sha256([19; 32]),
                &parents,
                &[],
            )),
        ];
        let attachments = [ProvenanceLocalId::new(0), ProvenanceLocalId::new(1)];
        let mut owners = vec![
            ProfileOwner::Scene,
            ProfileOwner::SyntheticRoot,
            ProfileOwner::Node(NodeLocalId::new(0)),
            ProfileOwner::LogicalMesh(0),
            ProfileOwner::Primitive(0),
            ProfileOwner::Primitive(1),
        ];
        owners.extend((0..attributes.len() as u32).map(ProfileOwner::Attribute));
        owners.push(ProfileOwner::MeshInstance(0));
        let owners: Vec<_> = owners
            .into_iter()
            .map(|owner| OwnerProvenanceAuthoring::new(owner, &attachments))
            .collect();
        let keys = [
            "test:scene",
            "test:normal-label",
            "test:tangent-bytes",
            "test:tangent-weight",
            "test:uv-negative-zero",
            "test:precision-next",
            "test:second-bits",
            "test:second-label",
        ]
        .map(|key| NamespacedName::new(key).expect("metadata key"));
        let media_type = ExtensionMediaType::new("application/octet-stream").expect("media type");
        let discarded = [0xde, 0xad, 0xbe, 0xef];
        let mut metadata = vec![MetadataAuthoringEntry::new(
            ProfileOwner::Scene,
            &keys[0],
            ExtensionValueRef::U64(73),
        )];
        if self.authored_normals {
            metadata.push(MetadataAuthoringEntry::new(
                ProfileOwner::Attribute(1),
                &keys[1],
                ExtensionValueRef::String("authored α normal"),
            ));
        }
        metadata.extend([
            MetadataAuthoringEntry::new(
                ProfileOwner::Attribute(tangent),
                &keys[2],
                ExtensionValueRef::Bytes {
                    media_type: &media_type,
                    payload: &discarded,
                },
            ),
            MetadataAuthoringEntry::new(
                ProfileOwner::Attribute(tangent),
                &keys[3],
                ExtensionValueRef::F64(123.125),
            ),
            MetadataAuthoringEntry::new(
                ProfileOwner::Attribute(uv),
                &keys[4],
                ExtensionValueRef::F64(-0.0),
            ),
            MetadataAuthoringEntry::new(
                ProfileOwner::Attribute(custom),
                &keys[5],
                ExtensionValueRef::F64(PRECISE),
            ),
            MetadataAuthoringEntry::new(
                ProfileOwner::Attribute(second_start + 1),
                &keys[6],
                ExtensionValueRef::F64(SECOND_PRECISE),
            ),
            MetadataAuthoringEntry::new(
                ProfileOwner::Attribute(second_start + 1),
                &keys[7],
                ExtensionValueRef::String("second primitive β"),
            ),
        ]);
        let space = SceneSpace::UNKNOWN;
        let snapshots = [ResourceSnapshotAuthoring::snapshotted(
            scope.primary_bytes(),
        )];
        scope.submit_scene(SceneAuthoringBatch::new(
            SpatialAuthoringBatch::new(&space, &[], &[]),
            NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
            AppearanceAuthoringBatch::empty(),
            GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &instances),
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
                MetadataAuthoringBatch::new(&metadata),
            )
            .with_resource_snapshots(&snapshots),
        ))
    }
}

fn import_fixture(authored_normals: bool) -> ImportReport {
    let mut registry = RegistryBuilder::new();
    registry
        .register(MetadataImporter {
            authored_normals,
            resource: ResourceIdentity::new(PATH).expect("document resource identity"),
            format: FormatId::new(FORMAT).expect("fixture format"),
        })
        .expect("metadata fixture registration");
    let registry = registry
        .freeze(RegistryBuildLimits::default())
        .expect("fixture registry");
    let request = ImportRequest::new().with_caller_layer(ImportRequestLayer::new(
        ImportConfigOverlay::default().with_format_selection(FormatSelection::Forced(
            FormatId::new(FORMAT).expect("forced fixture format"),
        )),
    ));
    let request = registry
        .compile_request(&request, OptionCompileLimits::default())
        .expect("fixture import request");
    let report = import_bytes(
        &request,
        AssetPath::new(PATH).expect("fixture asset path"),
        SOURCE,
        ImportOperationControl::default(),
    )
    .expect("explicit metadata Scene must satisfy the import resource/source closure");
    assert_eq!(report.ledger().current_live_bytes(), 0);
    assert_eq!(report.ledger().active_reservations(), 0);
    report
}

fn snapshot(scene: &ValidatedScene) -> SceneSnapshot {
    let snapshot = scene
        .capture_snapshot(
            SceneSnapshotLimits::try_new(64 * 1024, 256, 1_000_000).expect("snapshot limits"),
        )
        .expect("complete public Scene snapshot");
    assert!(!snapshot.preview_bytes_truncated());
    assert!(!snapshot.preview_items_truncated());
    snapshot
}

fn target(scene: &ValidatedScene) -> PrimitiveId {
    scene
        .meshes()
        .next()
        .expect("fixture mesh")
        .primitives()
        .next()
        .expect("target primitive")
        .id()
}

fn metadata_rows(snapshot: &SceneSnapshot) -> impl Iterator<Item = &str> {
    snapshot
        .preview()
        .lines()
        .filter(|line| line.starts_with("  profile_metadata["))
}

fn metadata_row<'a>(snapshot: &'a SceneSnapshot, key: &str) -> &'a str {
    let marker = format!(" key={key} ");
    let mut rows = metadata_rows(snapshot).filter(|line| line.contains(&marker));
    let row = rows
        .next()
        .unwrap_or_else(|| panic!("missing metadata key {key}"));
    assert!(rows.next().is_none(), "duplicate metadata key {key}");
    row
}

fn assert_attribute_metadata(snapshot: &SceneSnapshot, key: &str, owner: u32, value: &str) {
    let row = metadata_row(snapshot, key);
    assert!(
        row.contains(&format!(" owner_kind=12 owner_index={owner} key={key} ")),
        "{row}"
    );
    assert!(row.ends_with(value), "{row}");
}

fn assert_retained_metadata(snapshot: &SceneSnapshot, had_normal: bool) {
    // Metadata currently has a public bounded snapshot view. Require its complete preview and
    // every surviving row, including exact floating-point bits, rather than only total counts.
    assert_eq!(
        snapshot.profile_metadata_entry_count(),
        if had_normal { 6 } else { 5 }
    );
    assert_eq!(
        metadata_rows(snapshot).count(),
        snapshot.profile_metadata_entry_count()
    );
    assert!(!snapshot.preview().contains(" key=test:tangent-"));
    let scene = metadata_row(snapshot, "test:scene");
    assert!(scene.contains(" owner_kind=0 owner_index=0 key=test:scene "));
    assert!(scene.ends_with("value_kind=2 u64=73"));
    if had_normal {
        assert_attribute_metadata(
            snapshot,
            "test:normal-label",
            1,
            "value_kind=4 string=authored α normal",
        );
    } else {
        assert!(!snapshot.preview().contains(" key=test:normal-label "));
    }
    for (key, owner, value) in [
        ("test:uv-negative-zero", 2, -0.0_f64),
        ("test:precision-next", 4, PRECISE),
        ("test:second-bits", 6, SECOND_PRECISE),
    ] {
        assert_attribute_metadata(
            snapshot,
            key,
            owner,
            &format!("value_kind=3 f64_bits=0x{:016x}", value.to_bits()),
        );
    }
    assert_attribute_metadata(
        snapshot,
        "test:second-label",
        6,
        "value_kind=4 string=second primitive β",
    );
}

fn assert_f32_bits(actual: &[f32], expected: &[f32]) {
    assert_eq!(
        actual
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        expected
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>()
    );
}

fn assert_f64_bits(actual: &[f64], expected: &[f64]) {
    assert_eq!(
        actual
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        expected
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>()
    );
}

fn assert_output_geometry(scene: &ValidatedScene, output_target: PrimitiveId) {
    assert_eq!(scene.attribute_count(), 7);
    let primitive = scene
        .primitive(output_target)
        .expect("new-generation target");
    assert!(
        primitive
            .attributes()
            .map(|attribute| attribute.semantic())
            .eq([
                AttributeSemanticRef::Position,
                AttributeSemanticRef::Normal,
                AttributeSemanticRef::Texcoord(0),
                AttributeSemanticRef::SmoothingGroup,
                AttributeSemanticRef::Custom("test:precision"),
            ])
    );
    assert_eq!(primitive.indices(), Some(INDICES.as_slice()));
    assert_eq!(
        (
            primitive.vertex_count(),
            primitive.corner_count(),
            primitive.face_count()
        ),
        (4, 6, 2)
    );
    let AttributeData::DenseF32 { lanes: 3, values } = primitive.positions().unwrap().data() else {
        panic!("retained position payload");
    };
    assert_f32_bits(values, &POSITIONS);
    let normals = primitive
        .normals(AttributeDomain::Corner)
        .expect("generated normals");
    assert_eq!(normals.domain(), AttributeDomain::Corner);
    let AttributeData::DenseF32 { lanes: 3, values } = normals.data() else {
        panic!("generated F32 normals");
    };
    assert_eq!(values.len(), 18);
    for normal in values.chunks_exact(3) {
        assert_eq!(normal, &[0.0, 0.0, 1.0]);
    }
    let AttributeData::DenseF32 { lanes: 2, values } = primitive
        .texcoords(0, AttributeDomain::Corner)
        .unwrap()
        .data()
    else {
        panic!("retained UV payload");
    };
    assert_f32_bits(values, &TEXCOORDS);
    let AttributeData::DenseU32 { lanes: 1, values } = primitive
        .attribute(AttributeSemanticRef::SmoothingGroup, AttributeDomain::Face)
        .unwrap()
        .data()
    else {
        panic!("retained Face smoothing groups");
    };
    assert_eq!(values, &[7, 7]);
    let AttributeData::DenseF64 { lanes: 1, values } = primitive
        .attribute(
            AttributeSemanticRef::Custom("test:precision"),
            AttributeDomain::Vertex,
        )
        .unwrap()
        .data()
    else {
        panic!("retained F64 custom attribute");
    };
    assert_f64_bits(values, &CUSTOM);
    let second = primitive
        .mesh()
        .primitives()
        .nth(1)
        .expect("following primitive");
    assert!(
        second
            .attributes()
            .map(|attribute| attribute.semantic())
            .eq([
                AttributeSemanticRef::Position,
                AttributeSemanticRef::Custom("test:precision"),
            ])
    );
    assert_eq!(second.indices(), None);
    let AttributeData::DenseF32 { lanes: 3, values } = second.positions().unwrap().data() else {
        panic!("following primitive positions");
    };
    assert_f32_bits(values, &SECOND_POSITIONS);
    let AttributeData::DenseF64 { lanes: 1, values } = second
        .attribute(
            AttributeSemanticRef::Custom("test:precision"),
            AttributeDomain::Vertex,
        )
        .unwrap()
        .data()
    else {
        panic!("following primitive custom values");
    };
    assert_f64_bits(values, &SECOND_CUSTOM);
}

fn assert_finished(resources: &PostProcessResourceEvidence) {
    assert_eq!(resources.current_live_bytes(), 0);
    assert_eq!(resources.active_reservations(), 0);
    assert!(resources.consumed_work_items() > 0);
}

fn assert_changed_report(
    report: &PostProcessReport,
    scene: &ValidatedScene,
    input: PrimitiveId,
    options: NormalOptions,
    replaced: bool,
    removed: bool,
    removed_metadata: u32,
) -> PrimitiveId {
    assert_eq!(report.disposition(), PostProcessRunDisposition::Changed);
    assert_eq!(
        report.operation(),
        PostProcessOperation::GenerateNormals {
            target: input,
            options
        }
    );
    assert_eq!(report.input_generation(), input.generation());
    assert_eq!(report.output_generation(), scene.generation());
    assert_ne!(report.input_generation(), report.output_generation());
    assert!(report.matches_scene(scene));
    assert_finished(report.resources());
    assert!(report.resources().committed_canonical_bytes() > 0);
    let normal = report
        .normal_generation()
        .expect("published normal effects");
    assert_eq!(normal.input_target(), input);
    assert_eq!(normal.output_target().generation(), scene.generation());
    assert_eq!(normal.mode(), options.mode);
    assert_eq!(normal.replaced_authored_normals(), replaced);
    assert_eq!(normal.removed_tangents(), removed);
    assert_eq!(normal.removed_metadata_entries(), removed_metadata);
    assert_eq!(normal.output_normal_attribute_index(), 1);
    normal.output_target()
}

#[test]
fn default_tangent_rejection_returns_the_original_scene_and_all_metadata() {
    for authored_normals in [false, true] {
        let (scene, import_evidence) = import_fixture(authored_normals).into_parts();
        let before = snapshot(&scene);
        let generation = scene.generation();
        let input = target(&scene);
        let options = if authored_normals {
            NormalOptions {
                existing: ExistingNormals::Replace,
                ..NormalOptions::default()
            }
        } else {
            NormalOptions::default()
        };
        assert_eq!(options.tangents, TangentHandling::Reject);
        let failure = generate_normals(scene, input, options, PostProcessControl::default())
            .expect_err("generating normals requires explicit Tangent removal");
        assert_eq!(failure.input().generation(), generation);
        assert!(import_evidence.matches_scene(failure.input()));
        assert_eq!(snapshot(failure.input()), before);
        assert!(
            failure
                .input()
                .primitive(input)
                .unwrap()
                .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Vertex)
                .is_some()
        );
        let PostProcessRunError::Normals { failure: normal } = failure.error() else {
            panic!("typed normal-generation rejection");
        };
        assert!(matches!(
            normal.cause(),
            NormalCause::TangentsRequireRemoval
        ));
        let resources = normal
            .resources()
            .expect("actual terminal resource evidence");
        assert_finished(resources);
        assert_eq!(resources.committed_canonical_bytes(), 0);
        let (scene, error) = failure.into_parts();
        drop(error);
        drop(import_evidence);
        assert_eq!(snapshot(&scene), before);
    }
}

#[test]
fn replace_and_remove_preserves_other_metadata_and_accepts_a_second_new_target_operation() {
    let (scene, import_evidence) = import_fixture(true).into_parts();
    let before = snapshot(&scene);
    assert_eq!(before.profile_resource_count(), 1);
    assert_eq!(before.profile_source_count(), 1);
    assert_eq!(before.profile_provenance_node_count(), 2);
    assert_eq!(before.profile_operation_count(), 1);
    assert_eq!(before.attribute_count(), 8);
    assert_eq!(before.profile_metadata_entry_count(), 8);
    assert_attribute_metadata(
        &before,
        "test:uv-negative-zero",
        3,
        "value_kind=3 f64_bits=0x8000000000000000",
    );
    assert_attribute_metadata(
        &before,
        "test:second-label",
        7,
        "value_kind=4 string=second primitive β",
    );
    let input = target(&scene);
    let options = NormalOptions {
        existing: ExistingNormals::Replace,
        tangents: TangentHandling::Remove,
        ..NormalOptions::default()
    };
    let (scene, first_report) =
        generate_normals(scene, input, options, PostProcessControl::default())
            .expect("explicit replacement/removal must publish")
            .into_parts();
    let first_target = assert_changed_report(&first_report, &scene, input, options, true, true, 2);
    assert!(!import_evidence.matches_scene(&scene));
    let first_snapshot = snapshot(&scene);
    assert_eq!(
        first_snapshot.attribute_count(),
        before.attribute_count() - 1
    );
    assert_eq!(
        first_snapshot.profile_owner_count(),
        before.profile_owner_count() - 1
    );
    assert_eq!(
        first_snapshot.profile_provenance_node_count(),
        before.profile_provenance_node_count() + 2
    );
    assert_eq!(
        first_snapshot.profile_operation_count(),
        before.profile_operation_count() + 2
    );
    assert_retained_metadata(&first_snapshot, true);
    assert_output_geometry(&scene, first_target);
    drop(import_evidence);

    let options = NormalOptions {
        mode: NormalMode::Flat,
        existing: ExistingNormals::Replace,
        ..NormalOptions::default()
    };
    let (scene, second_report) =
        generate_normals(scene, first_target, options, PostProcessControl::default())
            .expect("the newly published target must support another operation")
            .into_parts();
    let second_target = assert_changed_report(
        &second_report,
        &scene,
        first_target,
        options,
        true,
        false,
        0,
    );
    assert!(!first_report.matches_scene(&scene));
    assert_eq!(
        first_report.normal_generation().unwrap().output_target(),
        first_target
    );
    assert_eq!(
        first_report
            .normal_generation()
            .unwrap()
            .removed_metadata_entries(),
        2
    );
    let second_snapshot = snapshot(&scene);
    assert_eq!(
        second_snapshot.profile_provenance_node_count(),
        first_snapshot.profile_provenance_node_count() + 2
    );
    assert!(metadata_rows(&first_snapshot).eq(metadata_rows(&second_snapshot)));
    assert_retained_metadata(&second_snapshot, true);
    drop(first_report);
    drop(second_report);
    assert_output_geometry(&scene, second_target);
    assert_retained_metadata(&snapshot(&scene), true);
}

#[test]
fn add_and_remove_keeps_the_attribute_count_and_retains_a_detached_effect_report() {
    let (scene, import_evidence) = import_fixture(false).into_parts();
    let before = snapshot(&scene);
    let input = target(&scene);
    assert!(
        scene
            .primitive(input)
            .unwrap()
            .normals(AttributeDomain::Corner)
            .is_none()
    );
    assert_eq!(before.attribute_count(), 7);
    assert_eq!(before.profile_metadata_entry_count(), 7);
    let options = NormalOptions {
        tangents: TangentHandling::Remove,
        ..NormalOptions::default()
    };
    let (scene, report) = generate_normals(scene, input, options, PostProcessControl::default())
        .expect("adding Normal and removing Tangent must keep the total attribute count")
        .into_parts();
    let output = assert_changed_report(&report, &scene, input, options, false, true, 2);
    let after = snapshot(&scene);
    assert_eq!(after.attribute_count(), before.attribute_count());
    assert_eq!(after.profile_owner_count(), before.profile_owner_count());
    assert_retained_metadata(&after, false);
    assert_output_geometry(&scene, output);
    drop(scene);
    drop(import_evidence);
    let effects = report
        .normal_generation()
        .expect("report survives both Scene owners");
    assert_eq!(effects.input_target(), input);
    assert_eq!(effects.output_target(), output);
    assert_eq!(effects.output_normal_attribute_index(), 1);
    assert_eq!(effects.removed_metadata_entries(), 2);
    assert!(effects.removed_tangents());
    assert!(!effects.replaced_authored_normals());
    assert_finished(report.resources());
    assert!(report.resources().published_outcome_evidence_bytes() > 0);
}

#[test]
fn default_preserve_keeps_authored_normal_tangent_and_metadata_unchanged() {
    let (scene, import_evidence) = import_fixture(true).into_parts();
    let before = snapshot(&scene);
    let input = target(&scene);
    let (scene, report) = generate_normals(
        scene,
        input,
        NormalOptions::default(),
        PostProcessControl::default(),
    )
    .expect("preserving authored normals does not invalidate Tangent")
    .into_parts();
    assert_eq!(scene.generation(), input.generation());
    assert_eq!(report.disposition(), PostProcessRunDisposition::Unchanged);
    assert!(report.normal_generation().is_none());
    assert!(import_evidence.matches_scene(&scene));
    assert_eq!(snapshot(&scene), before);
    assert_finished(report.resources());
    assert_eq!(report.resources().committed_canonical_bytes(), 0);
    drop(report);
    drop(import_evidence);
    assert_eq!(snapshot(&scene), before);
}
