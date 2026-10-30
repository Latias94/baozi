use super::*;
use crate::parser::model::{AttributeStorage, LogicalMeshStorage, PrimitiveStorage};
use crate::parser::{ContentError, owned_name};
use crate::source_map::{GltfSourceMap, SourceEntityKind};
use baozi_import::{
    DetectionOutcome, DetectionResult, DetectionScope, FormatDescriptor, FormatImporter,
    FormatSelection, ImportConfigOverlay, ImportOperationControl, ImportRequest,
    ImportRequestLayer, OptionCompileLimits, ParseResult, Probe, ReadConfidence, ReadHint,
    RegistryBuildLimits, RegistryBuilder, import_asset,
};
use baozi_io::{AssetPath, MemoryAssetIo};
use std::sync::Arc;

const FIXTURE_PRIMARY: &[u8] =
    br#"{"fixture":true,"nodes":[{}],"meshes":[{"primitives":[{}]}],"accessors":[{},{}]}"#;

#[derive(Clone, Copy, Default)]
enum FixtureFault {
    #[default]
    None,
    MissingNodeSource,
    MissingNodeOwner,
    InspectSourceMap,
}

struct HandBuiltModel(FixtureFault);

impl FormatImporter for HandBuiltModel {
    fn descriptor(&self) -> FormatDescriptor {
        crate::format_descriptor()
    }

    fn detect(
        &self,
        _: &Probe<'_>,
        _: &ReadHint,
        _: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
        build_fixture(scope, self.0)
            .and_then(|model| submit(scope, &model))
            .map_err(|error| match error {
                ContentError::Scope(failure) => failure,
                error => scope.format_failure(error.into_format_failure()),
            })
    }
}

fn values<'scope, T: Copy>(
    scope: &'scope ParseScope<'_>,
    values: &[T],
) -> Result<MeteredVec<'scope, T>> {
    let mut output = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "normalized-fixture",
    );
    output.try_extend_from_slice(values)?;
    Ok(output)
}

fn one<'scope, T>(scope: &'scope ParseScope<'_>, value: T) -> Result<MeteredVec<'scope, T>> {
    let mut output = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "normalized-fixture",
    );
    output.try_push(value)?;
    Ok(output)
}

fn build_fixture<'scope>(
    scope: &'scope ParseScope<'_>,
    fault: FixtureFault,
) -> Result<NormalizedGltf<'scope>> {
    use source_profile::{DOCUMENT, SelectedSourceProfile, entity};
    let map = GltfSourceMap::from_primary_for_test(scope)?;
    if matches!(fault, FixtureFault::InspectSourceMap) {
        inspect_source_inventory(scope, &map)?;
    }
    let node_source = entity(SourceEntityKind::Node, None, 0);
    let mesh_source = entity(SourceEntityKind::Mesh, None, 0);
    let primitive_source = entity(SourceEntityKind::Primitive, Some(0), 0);
    let position_source = entity(SourceEntityKind::Accessor, None, 0);
    let delta_source = entity(SourceEntityKind::Accessor, None, 1);
    let inventory = [
        DOCUMENT,
        node_source,
        mesh_source,
        primitive_source,
        position_source,
        delta_source,
    ];
    let mut sources = SelectedSourceProfile::from_source_map(scope, &map, |id| {
        inventory.contains(&id)
            && !(matches!(fault, FixtureFault::MissingNodeSource) && id == node_source)
    })?;
    for (owner, source) in [
        (ProfileOwner::Scene, DOCUMENT),
        (ProfileOwner::SyntheticRoot, DOCUMENT),
        (ProfileOwner::Node(NodeLocalId::new(0)), node_source),
        (ProfileOwner::LogicalMesh(0), mesh_source),
        (ProfileOwner::Primitive(0), primitive_source),
        (ProfileOwner::Attribute(0), primitive_source),
        (ProfileOwner::Attribute(0), position_source),
        (ProfileOwner::MeshInstance(0), node_source),
        (ProfileOwner::MeshInstance(0), mesh_source),
        (ProfileOwner::MorphDefinition(0), mesh_source),
        (ProfileOwner::MorphDelta(0), primitive_source),
        (ProfileOwner::MorphDelta(0), delta_source),
    ] {
        if matches!(fault, FixtureFault::MissingNodeOwner)
            && owner == ProfileOwner::Node(NodeLocalId::new(0))
        {
            continue;
        }
        sources.link(scope, owner, source)?;
    }
    sources.finish(scope)?;
    let positions = AttributeStorage {
        semantic: AttributeSemantic::Position,
        lanes: 3,
        values: values(scope, &[0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 3.0, 0.0])?,
        provenance: 0,
    };
    let primitive = PrimitiveStorage {
        topology: GeometryTopologyAuthoring::TriangleList,
        has_indices: true,
        indices: values(scope, &[2, 0, 1])?,
        material: None,
        attributes: one(scope, positions)?,
        influence_offsets: values(scope, &[])?,
        influences: values(scope, &[])?,
        provenance_offset: 0,
        provenance_count: 1,
    };
    let appearance = AppearanceStorage {
        images: scope.metered_vec(ImportStorageClass::FormatNormalization, "fixture-images"),
        encoded_image_payloads: scope
            .metered_vec(ImportStorageClass::FormatNormalization, "fixture-images"),
        samplers: scope.metered_vec(ImportStorageClass::FormatNormalization, "fixture-samplers"),
        sampler_provenance: values(scope, &[])?,
        textures: scope.metered_vec(ImportStorageClass::FormatNormalization, "fixture-textures"),
        texture_provenance: values(scope, &[])?,
        texture_names: scope.metered_vec(
            ImportStorageClass::FormatNormalization,
            "fixture-texture-names",
        ),
        bindings: scope.metered_vec(ImportStorageClass::FormatNormalization, "fixture-bindings"),
        materials: scope.metered_vec(ImportStorageClass::FormatNormalization, "fixture-materials"),
        image_ids: values(scope, &[])?,
        sampler_ids: values(scope, &[])?,
        texture_ids: values(scope, &[])?,
        default_sampler: None,
        material_ids: values(scope, &[])?,
    };
    let mesh = LogicalMeshStorage {
        name: owned_name(scope, Some("hand-built mesh"), "fixture-mesh-name")?,
        primitives: one(scope, primitive)?,
    };
    let node_id = NodeLocalId::new(0);
    let geometry = GeometryStorage {
        logical_meshes: one(scope, mesh)?,
        logical_mesh_ids: values(scope, &[Some(LogicalMeshLocalId::new(0))])?,
        primitive_provenance: values(scope, &[0])?,
    };
    let nodes = NodeStorage {
        roots: values(scope, &[node_id])?,
        nodes: values(
            scope,
            &[NodeAuthoringNode::new(
                Presence::Absent,
                AuthoredTransform::identity_trs(),
                Presence::Absent,
                Presence::Absent,
            )],
        )?,
        names: one(
            scope,
            owned_name(scope, Some("手工节点"), "fixture-node-name")?,
        )?,
        node_ids: values(scope, &[Some(node_id)])?,
        mesh_instances: one(
            scope,
            MeshInstanceStorage {
                authored: GeometryMeshInstanceAuthoring::new(node_id, LogicalMeshLocalId::new(0)),
                skin_index: None,
                source_node_index: 0,
            },
        )?,
    };
    let morphs = morph::MorphStorage {
        definitions: values(
            scope,
            &[MorphTargetDefinitionAuthoring::new(
                MorphTargetIdentity::new(0),
                Presence::Absent,
            )],
        )?,
        logical_meshes: one(
            scope,
            morph::LogicalMorphStorage {
                definitions: AuthoringRange::new(0, 1),
                weights: values(scope, &[0.5])?,
            },
        )?,
        primitives: one(scope, 0..1)?,
        targets: one(scope, morph::MorphTargetStorage { deltas: 0..1 })?,
        deltas: one(
            scope,
            morph::MorphDeltaStorage {
                semantic: MorphDeltaSemantic::Position,
                values: values(scope, &[0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0, 3.0])?,
                provenance: 0,
            },
        )?,
    };
    Ok(NormalizedGltf {
        sources,
        appearance,
        geometry,
        cameras: CameraStorage {
            authored: scope.metered_vec(ImportStorageClass::FormatNormalization, "fixture-cameras"),
            names: scope.metered_vec(
                ImportStorageClass::FormatNormalization,
                "fixture-camera-names",
            ),
            camera_ids: values(scope, &[])?,
        },
        nodes,
        skins: SkinStorage {
            skins: scope.metered_vec(ImportStorageClass::FormatNormalization, "fixture-skins"),
            skin_ids: values(scope, &[])?,
        },
        instance_deformations: one(
            scope,
            InstanceDeformationStorage {
                skin: Presence::Absent,
                weights: Some(values(scope, &[0.75])?),
            },
        )?,
        buffers: BufferResources {
            values: scope.metered_vec(ImportStorageClass::FormatNormalization, "fixture-buffers"),
        },
        external_images: ExternalImageResources {
            values: scope.metered_vec(ImportStorageClass::FormatNormalization, "fixture-images"),
            resource_offset: 1,
        },
        animations: animation::AnimationStorage::empty(scope),
        morphs,
    })
}

fn import_fixture(
    primary: &[u8],
    fault: FixtureFault,
) -> std::result::Result<
    baozi_import::ImportResult<baozi_import::ImportReport>,
    Box<dyn std::error::Error>,
> {
    let mut registry = RegistryBuilder::new();
    registry.register(HandBuiltModel(fault))?;
    let request = registry
        .freeze(RegistryBuildLimits::default())?
        .compile_request(
            &ImportRequest::from_layers(
                ImportRequestLayer::default(),
                ImportRequestLayer::new(
                    ImportConfigOverlay::default()
                        .with_format_selection(FormatSelection::Forced(FormatId::new(FORMAT_ID)?)),
                ),
            ),
            OptionCompileLimits::default(),
        )?;
    let io = MemoryAssetIo::new();
    let path = AssetPath::new("hand-built.gltf")?;
    io.try_insert(path.clone(), Arc::<[u8]>::from(primary))?;
    Ok(import_asset(
        &request,
        &io,
        path,
        ImportOperationControl::default(),
    ))
}

#[test]
fn canonical_authoring_accepts_a_hand_built_model_without_a_decoder()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let report = import_fixture(FIXTURE_PRIMARY, FixtureFault::None)??;
    let scene = report.scene();
    let mesh = scene.meshes().next().unwrap();
    assert_eq!(mesh.name(), Some("hand-built mesh"));
    assert_eq!(mesh.default_morph_weights(), &[0.5]);
    assert_eq!(
        mesh.morph_target_definitions()
            .next()
            .unwrap()
            .stable_identity(),
        0
    );
    let primitive = mesh.primitives().next().unwrap();
    assert_eq!(primitive.indices(), Some([2, 0, 1].as_slice()));
    assert_eq!(primitive.vertex_count(), 3);
    assert_eq!(
        primitive
            .morph_targets()
            .next()
            .unwrap()
            .deltas()
            .next()
            .unwrap()
            .data()
            .value(2),
        Some(Vec3::new(0.0, 0.0, 3.0))
    );
    let instance = scene.mesh_instances().next().unwrap();
    assert_eq!(instance.morph_weights(), Some([0.75].as_slice()));
    assert_eq!(
        scene
            .nodes()
            .find(|node| node.name() == Some("手工节点"))
            .unwrap()
            .name(),
        Some("手工节点")
    );
    assert!(report.diagnostics().is_empty());
    let snapshot = scene.capture_snapshot(baozi_import::SceneSnapshotLimits::try_new(
        32 * 1024,
        256,
        1_000_000,
    )?)?;
    assert_eq!(snapshot.profile_resource_count(), 1);
    assert_eq!(snapshot.profile_source_count(), 6);
    assert_eq!(snapshot.profile_provenance_node_count(), 7);
    assert_eq!(snapshot.profile_operation_count(), 1);
    assert_eq!(snapshot.profile_evidence_count(), 1);
    assert_eq!(snapshot.profile_parent_reference_count(), 1);
    assert_eq!(snapshot.profile_attachment_count(), 19);
    Ok(())
}

#[test]
fn normalized_authoring_rejects_missing_required_source_ranges_and_owner_links()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let missing_source = br#"{"fixture":true,"meshes":[{"primitives":[{}]}],"accessors":[{},{}]}"#;
    for (primary, fault) in [
        (missing_source.as_slice(), FixtureFault::None),
        (FIXTURE_PRIMARY, FixtureFault::MissingNodeSource),
        (FIXTURE_PRIMARY, FixtureFault::MissingNodeOwner),
    ] {
        let result = import_fixture(primary, fault)?;
        assert!(
            result.is_err(),
            "an incomplete source inventory must not publish a Scene"
        );
    }
    Ok(())
}

fn inspect_source_inventory(scope: &ParseScope<'_>, map: &GltfSourceMap<'_>) -> Result<()> {
    use source_profile::{DOCUMENT, SelectedSourceProfile, entity};
    let mut sources = SelectedSourceProfile::from_source_map(scope, map, |id| {
        id.kind != SourceEntityKind::Node || id.source_index < 2
    })?;
    let raw_base = if scope.primary_bytes().starts_with(b"glTF") {
        20
    } else {
        0
    };
    let document = sources.required(scope, DOCUMENT)?;
    assert_eq!(document.entity.range.byte_offset, raw_base);
    assert_eq!(
        document.locator.schema_id().unwrap().name().as_str(),
        "org.baozi.gltf:source-object"
    );
    assert_eq!(document.locator.schema_id().unwrap().version().get(), 1);

    for kind in [
        SourceEntityKind::Primitive,
        SourceEntityKind::AnimationChannel,
        SourceEntityKind::AnimationSampler,
    ] {
        let left = sources.required(scope, entity(kind, Some(0), 0))?;
        let right = sources.required(scope, entity(kind, Some(1), 0))?;
        let left_value = left.locator.namespaced_value().unwrap().as_str();
        let right_value = right.locator.namespaced_value().unwrap().as_str();
        assert!(left_value.contains(";parent=0;index=0;"), "{left_value}");
        assert!(right_value.contains(";parent=1;index=0;"), "{right_value}");
        assert_ne!(left_value, right_value);
    }
    let node_zero = entity(SourceEntityKind::Node, None, 0);
    let node_one = entity(SourceEntityKind::Node, None, 1);
    assert!(
        sources
            .required(scope, entity(SourceEntityKind::Node, None, 2))
            .is_err()
    );
    let raw = scope.primary_bytes();
    let json = std::str::from_utf8(&raw[raw_base as usize..]).unwrap();
    let node_text = r#"{"name":"same"}"#;
    let first_node = json.find(node_text).unwrap();
    let second_node = first_node
        + node_text.len()
        + json[first_node + node_text.len()..]
            .find(node_text)
            .unwrap();
    for (source, offset) in [(node_zero, first_node), (node_one, second_node)] {
        let selected = sources.required(scope, source)?;
        assert_eq!(selected.entity.range.byte_offset, raw_base + offset as u64);
        assert_eq!(selected.entity.range.byte_length, node_text.len() as u64);
        let label = selected.locator.namespaced_value().unwrap().as_str();
        assert_eq!(
            label,
            format!(
                "kind=node;parent=-;index={};offset={};length={}",
                source.source_index,
                raw_base + offset as u64,
                node_text.len()
            )
        );
    }
    sources.link(scope, ProfileOwner::Node(NodeLocalId::new(1)), node_one)?;
    sources.link(scope, ProfileOwner::Node(NodeLocalId::new(0)), node_zero)?;
    sources.link(scope, ProfileOwner::Node(NodeLocalId::new(0)), node_zero)?;
    sources.finish(scope)?;
    assert_eq!(sources.link_count(), 2);
    assert_eq!(
        sources.for_owner(scope, ProfileOwner::Node(NodeLocalId::new(0)))?[0].source,
        node_zero
    );
    assert_ne!(
        sources.provenance(scope, 1, node_zero)?,
        sources.provenance(scope, 1, node_one)?
    );
    Ok(())
}

#[test]
fn selected_inventory_preserves_nested_parent_identity_and_glb_raw_offsets_without_a_decoder()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let json = br#"{"fixture":true,"nodes":[{"name":"same"},{"name":"same"},{"name":"omitted"}],"meshes":[{"primitives":[{},{}]},{"primitives":[{}]}],"accessors":[{},{}],"animations":[{"channels":[{},{}],"samplers":[{}]},{"channels":[{}],"samplers":[{}]}]}"#;
    let padded_length = json.len().div_ceil(4) * 4;
    let mut glb = Vec::new();
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(20 + padded_length as u32).to_le_bytes());
    glb.extend_from_slice(&(padded_length as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(json);
    glb.resize(20 + padded_length, b' ');
    for primary in [json.as_slice(), glb.as_slice()] {
        let report = import_fixture(primary, FixtureFault::InspectSourceMap)??;
        let snapshot =
            report
                .scene()
                .capture_snapshot(baozi_import::SceneSnapshotLimits::try_new(
                    32 * 1024,
                    256,
                    1_000_000,
                )?)?;
        assert_eq!(snapshot.profile_source_count(), 6);
    }
    Ok(())
}
