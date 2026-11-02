use baozi::{
    AttributeSemanticRef, CapabilityStatus, DigestAlgorithm, EntityMetadataOwner, FormatCapability,
    FormatDescriptor, FormatEncoding, FormatId, ImportReport, Importer, MetadataOwnerRef,
    MetadataValueRef, ProvenanceKindRef, ProvenanceLookupError, ProvenanceOperationKindRef,
    ProvenanceRef, ReadConfidence, SourceIdentityRef, SourceObjectKind, SourceObjectLocatorRef,
    TangentBasisRef, TangentEncodingRef, ValidatedScene,
};
use baozi_import::authoring::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, AttributeDomain, AttributeSemantic,
    AuthoredTransform, AuthoringRange, DeformationAuthoringBatch, EvidenceDigest,
    ExtensionMediaType, ExtensionValueRef, GeometryAttributeAuthoring, GeometryAuthoringBatch,
    GeometryLogicalMeshAuthoring, GeometryPrimitiveAuthoring, GeometryTopologyAuthoring,
    LineageAuthoringBatch, LogicalMorphAuthoring, MetadataAuthoringBatch, MetadataAuthoringEntry,
    NamespacedName, NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, NodeParent,
    ObservedResourceAuthoring, OperationEvidenceSummary, OperationKind, OptionsDigest,
    OwnerProvenanceAuthoring, Presence, PrimitiveDeformationAuthoring, ProfileAuthoringBatch,
    ProfileOwner, ProvenanceLocalId, ProvenanceNodeAuthoring, ProvenanceOperationAuthoring,
    ResourceIdentity, ResourceIdentityLocalId, SceneAuthoringBatch, SceneSpace,
    SessionOpaqueReason, SourceDisplayName, SourceIdentityAuthoring, SourceIdentityLocalId,
    SourceObjectLocator, SpatialAuthoringBatch,
};
use baozi_import::{
    CompiledOptions, DetectionOutcome, DetectionResult, DetectionScope, Format, NoFormatOptions,
    OptionCompileError, ParseContext, ParseResult, Probe, ReadHint, RegistryBuildScope,
};

const JSON: &[u8] = br#"{ "nested": [null, 1.00e+02, {"escape":"\u0061"}] }"#;

// Public plugin authoring gives the fixture polygons and tangents that no current format imports
// together. All observations below use the ordinary baozi facade, including after real edits.
struct ProvenanceFixture;

impl Format for ProvenanceFixture {
    type Options = NoFormatOptions;

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> std::result::Result<CompiledOptions<Self::Options>, OptionCompileError> {
        NoFormatOptions::compile_defaults(scope)
    }

    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(
            FormatId::new("org.baozi.provenance-fixture").unwrap(),
            "Provenance reader fixture",
        )
        .extensions(["provenance"])
        .encoding(FormatEncoding::Binary)
        .capabilities([(FormatCapability::Geometry, CapabilityStatus::Unknown)])
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

    fn parse(&self, _options: &Self::Options, scope: &mut ParseContext<'_>) -> ParseResult<()> {
        let roots = [NodeLocalId::new(1)];
        let nodes = [
            NodeAuthoringNode::new(
                Presence::Present(NodeParent::new(NodeLocalId::new(1), 0)),
                AuthoredTransform::identity_trs(),
                Presence::Absent,
                Presence::Absent,
            )
            .with_name("same"),
            NodeAuthoringNode::new(
                Presence::Absent,
                AuthoredTransform::identity_trs(),
                Presence::Absent,
                Presence::Absent,
            )
            .with_name("same"),
        ];
        let positions = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0];
        let normals = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
        let tangents = [
            1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0,
        ];
        let attributes: [_; 6] = std::array::from_fn(|index| {
            let (semantic, lanes, values): (_, _, &[f32]) = match index % 3 {
                0 => (AttributeSemantic::Position, 3, &positions),
                1 => (AttributeSemantic::Normal, 3, &normals),
                _ => (AttributeSemantic::Tangent, 4, &tangents),
            };
            GeometryAttributeAuthoring::f32(
                semantic,
                AttributeDomain::Vertex,
                lanes,
                values,
                Presence::Absent,
            )
        });
        let indices = [0, 1, 2, 3];
        let faces = [4];
        let primitives: [_; 2] = std::array::from_fn(|index| {
            GeometryPrimitiveAuthoring::new(
                GeometryTopologyAuthoring::Polygons(&faces),
                Presence::Present(&indices),
                baozi::MaterialAssignment::Unspecified,
                AuthoringRange::new(index as u32 * 3, 3),
            )
        });
        let meshes =
            [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 2)).with_name("same")];
        let logical_morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
        let deformations = [PrimitiveDeformationAuthoring::new(Presence::Absent, &[]); 2];
        let resource = ResourceIdentity::new("models/retained.provenance").unwrap();
        let resources = [ObservedResourceAuthoring::session_opaque(
            &resource,
            SessionOpaqueReason::CallerLabel,
        )];
        let format = FormatId::new("org.baozi.provenance-fixture").unwrap();
        let same_name = SourceDisplayName::new("same").unwrap();
        let locators = [
            SourceObjectLocator::indexed(SourceObjectKind::Primitive, 93),
            SourceObjectLocator::indexed(SourceObjectKind::Primitive, 17),
            SourceObjectLocator::indexed(SourceObjectKind::Document, 99),
        ];
        let sources = locators.each_ref().map(|locator| {
            SourceIdentityAuthoring::new(ResourceIdentityLocalId::new(0), &format, locator)
                .with_display_name(&same_name)
        });
        let generated = OperationKind::Generated;
        let merged = OperationKind::Merge;
        let parents = [ProvenanceLocalId::new(0), ProvenanceLocalId::new(2)];
        let evidence = [OperationEvidenceSummary::new(
            NamespacedName::new("test:shared").unwrap(),
            2,
            Some(EvidenceDigest::sha256([31; 32])),
        )
        .unwrap()];
        // Deliberately independent orders: retained node 0 -> source row 1 -> source index 17.
        let provenance = [
            ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(1)),
            ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(2)),
            ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(0)),
            ProvenanceNodeAuthoring::operation(ProvenanceOperationAuthoring::new(
                &generated,
                OptionsDigest::sha256([5; 32]),
                &[],
                &[],
            )),
            ProvenanceNodeAuthoring::operation(ProvenanceOperationAuthoring::new(
                &merged,
                OptionsDigest::blake3([7; 32]),
                &parents,
                &evidence,
            )),
        ];
        let shared = [ProvenanceLocalId::new(0)];
        let document = [ProvenanceLocalId::new(1)];
        let tangent = [ProvenanceLocalId::new(2)];
        let synthetic = [ProvenanceLocalId::new(3)];
        let merged_attachment = [ProvenanceLocalId::new(4)];
        let owners = [
            OwnerProvenanceAuthoring::new(ProfileOwner::Scene, &document),
            OwnerProvenanceAuthoring::new(ProfileOwner::SyntheticRoot, &synthetic),
            OwnerProvenanceAuthoring::new(ProfileOwner::Node(NodeLocalId::new(0)), &shared),
            OwnerProvenanceAuthoring::new(ProfileOwner::Node(NodeLocalId::new(1)), &tangent),
            OwnerProvenanceAuthoring::new(ProfileOwner::LogicalMesh(0), &merged_attachment),
            OwnerProvenanceAuthoring::new(ProfileOwner::Primitive(0), &merged_attachment),
            OwnerProvenanceAuthoring::new(ProfileOwner::Primitive(1), &merged_attachment),
            OwnerProvenanceAuthoring::new(ProfileOwner::Attribute(0), &shared),
            OwnerProvenanceAuthoring::new(ProfileOwner::Attribute(1), &shared),
            OwnerProvenanceAuthoring::new(ProfileOwner::Attribute(2), &tangent),
            OwnerProvenanceAuthoring::new(ProfileOwner::Attribute(3), &shared),
            OwnerProvenanceAuthoring::new(ProfileOwner::Attribute(4), &shared),
            OwnerProvenanceAuthoring::new(ProfileOwner::Attribute(5), &tangent),
        ];
        let key = NamespacedName::new("test:extras").unwrap();
        let media_type = ExtensionMediaType::new("application/json").unwrap();
        let metadata = [
            MetadataAuthoringEntry::for_source(
                SourceIdentityLocalId::new(0),
                &key,
                ExtensionValueRef::Bytes {
                    media_type: &media_type,
                    payload: JSON,
                },
            ),
            MetadataAuthoringEntry::for_source(
                SourceIdentityLocalId::new(1),
                &key,
                ExtensionValueRef::Bytes {
                    media_type: &media_type,
                    payload: b"null",
                },
            ),
            MetadataAuthoringEntry::new(
                ProfileOwner::Attribute(2),
                &key,
                ExtensionValueRef::String("canonical tangent annotation"),
            ),
        ];
        scope.submit_complete(SceneAuthoringBatch::new(
            SpatialAuthoringBatch::new(&SceneSpace::STANDARD_TARGET, &[], &[]),
            NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
            AppearanceAuthoringBatch::empty(),
            GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]),
            DeformationAuthoringBatch::new(&[], &[], &logical_morphs, &deformations, &[]),
            AnimationAuthoringBatch::empty(),
            ProfileAuthoringBatch::new(
                LineageAuthoringBatch::new(&resources, &sources, &provenance, &owners),
                MetadataAuthoringBatch::new(&metadata),
            ),
        ))
    }
}

fn fixture_report() -> ImportReport {
    let mut builder = Importer::builder();
    builder.register(ProvenanceFixture).unwrap();
    builder
        .build()
        .unwrap()
        .read_bytes("fixture.provenance", b"fixture")
        .unwrap()
}

#[test]
fn facade_keeps_undeclared_tangent_conventions_unknown_on_each_attribute() {
    let report = fixture_report();
    for primitive in report.scene().meshes().next().unwrap().primitives() {
        let tangent = primitive
            .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Vertex)
            .unwrap();
        let conventions = tangent.tangent_conventions().unwrap();
        assert_eq!(conventions.basis, TangentBasisRef::Unknown);
        assert_eq!(conventions.encoding, TangentEncodingRef::Unknown);
        assert!(
            primitive
                .attributes()
                .filter(|attribute| attribute.semantic() != AttributeSemanticRef::Tangent)
                .all(|attribute| attribute.tangent_conventions().is_none())
        );
    }
}

fn only_attachment(scene: &ValidatedScene, owner: EntityMetadataOwner) -> ProvenanceRef<'_> {
    let mut attachments = scene.owner_provenance(owner).unwrap();
    assert_eq!(attachments.len(), 1);
    attachments.next().unwrap()
}

fn source(node: ProvenanceRef<'_>) -> SourceIdentityRef<'_> {
    let ProvenanceKindRef::Source(source) = node.kind() else {
        panic!("expected a retained source leaf")
    };
    source
}

fn extras(source: SourceIdentityRef<'_>) -> &[u8] {
    let entry = source.metadata_entries().next().expect("source has extras");
    assert_eq!(entry.owner(), MetadataOwnerRef::SourceObject(source));
    let MetadataValueRef::Bytes {
        media_type,
        payload,
    } = entry.value()
    else {
        panic!("source extras are exact byte payloads")
    };
    assert_eq!(media_type, "application/json");
    payload
}

// Caller-owned traversal state and a fixed visit budget; the library follows immediate edges only.
fn source_ancestors(
    scene: &ValidatedScene,
    owner: EntityMetadataOwner,
) -> Vec<SourceIdentityRef<'_>> {
    let mut pending: Vec<_> = scene.owner_provenance(owner).unwrap().rev().collect();
    let mut sources = Vec::new();
    let mut visits = 0;
    while let Some(node) = pending.pop() {
        visits += 1;
        assert!(
            visits <= 64,
            "fixture ancestry exceeds the caller's visit budget"
        );
        match node.kind() {
            ProvenanceKindRef::Source(source) => sources.push(source),
            ProvenanceKindRef::Operation(_) => pending.extend(node.parents().rev()),
        }
    }
    sources
}

#[cfg(feature = "postprocess")]
fn assert_source_payloads(scene: &ValidatedScene, owner: EntityMetadataOwner) {
    let sources = source_ancestors(scene, owner);
    for (index, expected) in [(17, b"null".as_slice()), (93, JSON)] {
        let source = sources
            .iter()
            .copied()
            .find(|source| {
                source.locator()
                    == SourceObjectLocatorRef::Indexed {
                        kind: SourceObjectKind::Primitive,
                        index,
                    }
            })
            .expect("retained edges still reach the original primitive source");
        assert_eq!(extras(source), expected);
        assert_eq!(source.resource().identity(), "models/retained.provenance");
    }
    assert!(
        sources
            .iter()
            .all(|source| source.generation() == scene.generation())
    );
    // Appended edit segments have no evidence rows. The original nonempty summary remains readable.
    let merge = scene
        .provenance_nodes()
        .find_map(|node| match node.kind() {
            ProvenanceKindRef::Operation(operation)
                if operation.kind() == ProvenanceOperationKindRef::Merge =>
            {
                Some(operation)
            }
            _ => None,
        })
        .unwrap();
    let evidence = merge.evidence().next().unwrap();
    assert_eq!(evidence.code(), "test:shared");
    assert_eq!(
        evidence.digest(),
        Some((DigestAlgorithm::Sha256, [31; 32].as_slice()))
    );
}

#[test]
fn facade_joins_reordered_same_named_and_shared_sources_through_retained_edges() {
    let report = fixture_report();
    let scene = report.scene();
    assert_eq!(scene.provenance_node_count(), 5);
    assert_eq!(scene.source_identity_count(), 3);
    let root = scene.roots().next().unwrap();
    let child = root.children().next().unwrap();
    assert_eq!((root.name(), child.name()), (Some("same"), Some("same")));
    let root_source = source(only_attachment(scene, root.metadata_owner()));
    let child_source = source(only_attachment(scene, child.metadata_owner()));
    assert_eq!(root_source.display_name(), child_source.display_name());
    assert_eq!((root_source.index(), child_source.index()), (0, 1));
    assert_eq!(
        root_source.locator(),
        SourceObjectLocatorRef::Indexed {
            kind: SourceObjectKind::Primitive,
            index: 93,
        }
    );
    assert_eq!(
        child_source.locator(),
        SourceObjectLocatorRef::Indexed {
            kind: SourceObjectKind::Primitive,
            index: 17,
        }
    );
    assert_eq!(
        (extras(root_source), extras(child_source)),
        (JSON, b"null".as_slice())
    );
    assert_eq!(
        root_source.resource().identity(),
        "models/retained.provenance"
    );
    assert_eq!(root_source.resource().generation(), scene.generation());
    let mesh = scene.meshes().next().unwrap();
    let first = mesh.primitives().next().unwrap();
    let second = mesh.primitives().next_back().unwrap();
    let position_source = |primitive: baozi::PrimitiveRef<'_>| {
        only_attachment(scene, primitive.positions().unwrap().metadata_owner())
    };
    assert_eq!(position_source(first).id(), position_source(second).id());
    assert_eq!(source(position_source(first)), child_source);
    let ancestors = source_ancestors(scene, first.metadata_owner());
    assert_eq!(ancestors, [child_source, root_source]);
    assert_eq!(source_ancestors(scene, second.metadata_owner()), ancestors);
    let node = only_attachment(scene, first.metadata_owner());
    assert_eq!(node.depth(), 2);
    assert_eq!(node.parents().len(), 2);
    let ProvenanceKindRef::Operation(operation) = node.kind() else {
        panic!("merged primitive")
    };
    assert_eq!(operation.kind(), ProvenanceOperationKindRef::Merge);
    assert_eq!(
        operation.options_digest(),
        (DigestAlgorithm::Blake3, [7; 32].as_slice())
    );
    let evidence = operation.evidence().next().unwrap();
    assert_eq!(operation.evidence_count(), 1);
    assert_eq!(
        (evidence.code(), evidence.occurrence_count()),
        ("test:shared", 2)
    );
    assert_eq!(
        evidence.digest(),
        Some((DigestAlgorithm::Sha256, [31; 32].as_slice()))
    );
    assert_eq!(scene.provenance(node.id()).unwrap(), node);
    let generated = scene.provenance_nodes().nth(3).unwrap();
    assert_eq!(generated.parents().len(), 0);
    let ProvenanceKindRef::Operation(operation) = generated.kind() else {
        panic!("generated root")
    };
    assert_eq!(operation.kind(), ProvenanceOperationKindRef::Generated);
    assert_eq!(operation.evidence().len(), 0);
    let document = source(only_attachment(scene, scene.metadata_owner()));
    assert_eq!(document.metadata_entries().len(), 0);
    assert_eq!(scene.provenance_nodes().next().unwrap().parents().len(), 0);
}

#[test]
fn facade_rejects_tokens_from_another_scene_even_with_identical_source_identities() {
    let first = fixture_report();
    let second = fixture_report();
    let owner = first.scene().meshes().next().unwrap().metadata_owner();
    let id = only_attachment(first.scene(), owner).id();
    assert!(matches!(
        second.scene().owner_provenance(owner),
        Err(ProvenanceLookupError::GenerationMismatch { expected, actual })
            if expected == second.scene().generation() && actual == first.scene().generation()
    ));
    assert!(matches!(
        second.scene().provenance(id),
        Err(ProvenanceLookupError::GenerationMismatch { expected, actual })
            if expected == second.scene().generation() && actual == first.scene().generation()
    ));
}

#[cfg(feature = "postprocess")]
#[test]
fn facade_keeps_source_joins_after_triangulation_conversion_and_tangent_deletion() {
    use baozi::postprocess::{
        ExistingNormals, NormalMode, NormalOptions, PostProcessControl, PostProcessRunDisposition,
        SpaceConversionOptions, TangentPolicy, TrianglePreparationOptions, convert_scene_space,
        prepare_triangles, triangulate_primitive,
    };
    use baozi::{
        AttributeSemanticRef, Axis, FrontFaceWinding, Handedness, TargetSpace, TopologyKind,
    };

    let scene = fixture_report().into_parts().0;
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    let old_owner = primitive.metadata_owner();
    let old_node = only_attachment(&scene, old_owner).id();
    let target = primitive.id();
    let triangulated = triangulate_primitive(scene, target, PostProcessControl::default()).unwrap();
    assert_eq!(
        triangulated.disposition(),
        PostProcessRunDisposition::Changed
    );
    let scene = triangulated.into_parts().0;
    assert!(matches!(
        scene.owner_provenance(old_owner),
        Err(ProvenanceLookupError::GenerationMismatch { .. })
    ));
    assert!(matches!(
        scene.provenance(old_node),
        Err(ProvenanceLookupError::GenerationMismatch { .. })
    ));
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    assert_eq!(primitive.topology(), TopologyKind::TriangleList);
    let node = only_attachment(&scene, primitive.metadata_owner());
    let ProvenanceKindRef::Operation(operation) = node.kind() else {
        panic!("triangulation operation")
    };
    assert_eq!(operation.kind(), ProvenanceOperationKindRef::Split);
    assert_eq!(node.parents().len(), 1);
    assert_source_payloads(&scene, primitive.metadata_owner());

    let converted = convert_scene_space(
        scene,
        SpaceConversionOptions {
            target: TargetSpace::try_new(
                Handedness::Right,
                Axis::PositiveY,
                Axis::NegativeZ,
                0.5,
                FrontFaceWinding::CounterClockwise,
            )
            .unwrap(),
            source_assumptions: None,
        },
        PostProcessControl::default(),
    )
    .unwrap();
    assert_eq!(converted.disposition(), PostProcessRunDisposition::Changed);
    let scene = converted.into_parts().0;
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    let node = only_attachment(&scene, primitive.metadata_owner());
    let ProvenanceKindRef::Operation(operation) = node.kind() else {
        panic!("conversion operation")
    };
    assert_eq!(operation.kind(), ProvenanceOperationKindRef::Transform);
    assert_eq!(node.parents().len(), 2);
    let document = source(node.parents().next().unwrap());
    assert_eq!(
        document.locator(),
        SourceObjectLocatorRef::Indexed {
            kind: SourceObjectKind::Document,
            index: 99,
        }
    );
    assert_eq!(document.metadata_entries().len(), 0);
    assert_source_payloads(&scene, primitive.metadata_owner());
    let tangent_owner = primitive
        .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Vertex)
        .unwrap()
        .metadata_owner();
    let target = primitive.id();
    let generated = prepare_triangles(
        scene,
        &[target],
        TrianglePreparationOptions {
            normals: Some(NormalOptions {
                mode: NormalMode::Flat,
                existing: ExistingNormals::Replace,
            }),
            tangents: TangentPolicy::Remove,
            ..TrianglePreparationOptions::default()
        },
        PostProcessControl::default(),
    )
    .unwrap();
    assert_eq!(generated.disposition(), PostProcessRunDisposition::Changed);
    let scene = generated.into_parts().0;
    assert!(matches!(
        scene.owner_provenance(tangent_owner),
        Err(ProvenanceLookupError::GenerationMismatch { .. })
    ));
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    assert!(
        primitive
            .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Vertex)
            .is_none()
    );
    assert!(primitive.normals(AttributeDomain::Corner).is_some());
    assert!(
        scene
            .metadata_entries()
            .all(|entry| entry.value() != MetadataValueRef::String("canonical tangent annotation"))
    );
    let untouched = scene
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next_back()
        .unwrap();
    assert!(
        untouched
            .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Vertex)
            .is_some()
    );
    // Deleting canonical metadata can compact the shared payload pools. Exact source bytes and
    // relationships survive, but pointer identity is only meaningful within one generation.
    assert_source_payloads(&scene, primitive.metadata_owner());
    let normal = primitive.normals(AttributeDomain::Corner).unwrap();
    let node = only_attachment(&scene, normal.metadata_owner());
    let ProvenanceKindRef::Operation(operation) = node.kind() else {
        panic!("normal generation operation")
    };
    assert_eq!(operation.kind(), ProvenanceOperationKindRef::Generated);
}

#[cfg(feature = "format-gltf")]
#[test]
fn facade_follows_actual_gltf_source_locators_after_selected_node_reordering() {
    let document = br#"{
      "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[2]}],
      "nodes":[
        {"name":"same","extras": {"exact":1.00e+02}},
        {"name":"unselected","extras":false},
        {"name":"same","children":[0],"extras":null}
      ]
    }"#;
    let report = Importer::new()
        .unwrap()
        .read_bytes("reordered.gltf", document)
        .unwrap();
    let scene = report.scene();
    let root = scene.roots().next().unwrap();
    let child = root.children().next().unwrap();
    assert_eq!((root.name(), child.name()), (Some("same"), Some("same")));
    for (node, expected_index, expected_json) in [
        (root, "index=2;", b"null".as_slice()),
        (child, "index=0;", br#"{"exact":1.00e+02}"#.as_slice()),
    ] {
        let sources = source_ancestors(scene, node.metadata_owner());
        let object = sources
            .iter()
            .copied()
            .find(|source| {
                matches!(source.locator(), SourceObjectLocatorRef::Namespaced { value, .. }
                    if value.starts_with("kind=node;") && value.contains(expected_index))
            })
            .expect("node ancestry retains its selected original source object");
        assert_eq!(object.format(), "org.baozi.gltf");
        assert_eq!(extras(object), expected_json);
        assert_eq!(
            object.metadata_entries().next().unwrap().key(),
            "org.baozi.gltf:extras"
        );
    }
}
