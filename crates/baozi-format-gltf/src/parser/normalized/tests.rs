use super::*;
use crate::parser::model::{AttributeStorage, LogicalMeshStorage, PrimitiveStorage};
use crate::parser::{ContentError, owned_name};
use crate::source_map::{GltfSourceMap, SourceEntityKind};
use baozi_import::ParseServices;
use baozi_import::{
    DetectionOutcome, DetectionResult, DetectionScope, Format, FormatDescriptor, FormatSelection,
    ImportConfigOverlay, ImportOperationControl, ImportRequest, ImportRequestLayer,
    OptionCompileLimits, ParseResult, Probe, ReadConfidence, ReadHint, RegistryBuildLimits,
    RegistryBuilder,
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
    Light,
}

struct HandBuiltModel(FixtureFault);

impl Format for HandBuiltModel {
    type Options = baozi_import::NoFormatOptions;

    fn compile_defaults(
        &self,
        scope: &mut baozi_import::RegistryBuildScope<'_>,
    ) -> std::result::Result<
        baozi_import::CompiledOptions<Self::Options>,
        baozi_import::OptionCompileError,
    > {
        Self::Options::compile_defaults(scope)
    }

    fn descriptor(&self) -> FormatDescriptor {
        crate::format_descriptor()
    }

    fn detect(
        &self,
        _: &Probe<'_>,
        _: &ReadHint,
        _: &mut DetectionScope<'_, '_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, _: &Self::Options, context: &mut ParseContext<'_, '_>) -> ParseResult<()> {
        let result = context
            .with_execution(|scope| build_fixture(scope, self.0))
            .and_then(|model| submit(context, &model));
        match result {
            Ok(()) => Ok(()),
            Err(ContentError::Scope(failure)) => Err(failure),
            Err(error) => context
                .with_execution(|scope| Err(scope.format_failure(error.into_format_failure()))),
        }
    }
}

fn values<'scope, T: Copy>(
    scope: &mut ParseServices<'_, '_, 'scope>,
    values: &[T],
) -> Result<MeteredVec<'scope, T>> {
    let mut output = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "normalized-fixture",
    );
    output.try_extend_from_slice(scope, values)?;
    Ok(output)
}

fn one<'scope, T>(
    scope: &mut ParseServices<'_, '_, 'scope>,
    value: T,
) -> Result<MeteredVec<'scope, T>> {
    let mut output = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "normalized-fixture",
    );
    output.try_push(scope, value)?;
    Ok(output)
}

fn build_fixture<'scope>(
    scope: &mut ParseRoot<'_, '_, 'scope>,
    fault: FixtureFault,
) -> Result<NormalizedGltf<'scope>> {
    use source_profile::{DOCUMENT, OwnerSourceRun, SelectedSourceProfile, entity};
    let map = GltfSourceMap::from_primary_for_test(scope)?;
    if matches!(fault, FixtureFault::InspectSourceMap) {
        inspect_source_inventory(scope, &map)?;
    }
    let node_source = entity(SourceEntityKind::Node, None, 0);
    let mesh_source = entity(SourceEntityKind::Mesh, None, 0);
    let primitive_source = entity(SourceEntityKind::Primitive, Some(0), 0);
    let position_source = entity(SourceEntityKind::Accessor, None, 0);
    let delta_source = entity(SourceEntityKind::Accessor, None, 1);
    let light_source = entity(SourceEntityKind::Light, None, 0);
    let inventory = [
        DOCUMENT,
        node_source,
        mesh_source,
        primitive_source,
        position_source,
        delta_source,
        light_source,
    ];
    let mut sources = SelectedSourceProfile::from_source_map(scope, &map, |id| {
        inventory.contains(&id)
            && !(matches!(fault, FixtureFault::MissingNodeSource) && id == node_source)
    })?;
    let mut run = OwnerSourceRun::new(scope)?;
    let suspended = OwnerSourceRun::new(scope)?;
    for (owner, identities) in [
        (ProfileOwner::Scene, &[DOCUMENT][..]),
        (ProfileOwner::SyntheticRoot, &[DOCUMENT][..]),
        (ProfileOwner::Node(NodeLocalId::new(0)), &[node_source][..]),
        (ProfileOwner::LogicalMesh(0), &[mesh_source][..]),
        (ProfileOwner::Primitive(0), &[primitive_source][..]),
        (
            ProfileOwner::Attribute(0),
            &[primitive_source, position_source][..],
        ),
        (
            ProfileOwner::MeshInstance(0),
            &[node_source, mesh_source][..],
        ),
        (ProfileOwner::MorphDefinition(0), &[mesh_source][..]),
        (
            ProfileOwner::MorphDelta(0),
            &[primitive_source, delta_source][..],
        ),
    ] {
        if matches!(fault, FixtureFault::MissingNodeOwner)
            && owner == ProfileOwner::Node(NodeLocalId::new(0))
        {
            continue;
        }
        for &source in identities {
            sources.link(scope, &mut run, owner, source)?;
        }
        sources.complete_owner(scope, &mut run, owner)?;
    }
    if matches!(fault, FixtureFault::Light) {
        sources.link(scope, &mut run, ProfileOwner::Light(0), light_source)?;
        sources.complete_owner(scope, &mut run, ProfileOwner::Light(0))?;
    }
    sources.finish(scope, &run, &suspended)?;
    let positions = AttributeStorage {
        semantic: AttributeSemantic::Position,
        lanes: 3,
        values: values(
            &mut scope.services()?,
            &[0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 3.0, 0.0],
        )?,
        provenance: AccessorSources::ROOT,
    };
    let primitive = PrimitiveStorage {
        topology: GeometryTopologyAuthoring::TriangleList,
        has_indices: true,
        indices: values(&mut scope.services()?, &[2, 0, 1])?,
        material_assignment: MaterialAssignment::Unspecified,
        attributes: one(&mut scope.services()?, positions)?,
        influence_offsets: values(&mut scope.services()?, &[])?,
        influences: values(&mut scope.services()?, &[])?,
        provenance_offset: 0,
        provenance_count: 1,
    };
    let appearance = AppearanceStorage {
        images: scope
            .services()?
            .metered_vec(ImportStorageClass::FormatNormalization, "fixture-images"),
        image_names: scope.services()?.metered_vec(
            ImportStorageClass::FormatNormalization,
            "fixture-image-names",
        ),
        encoded_image_payloads: scope
            .services()?
            .metered_vec(ImportStorageClass::FormatNormalization, "fixture-images"),
        samplers: scope
            .services()?
            .metered_vec(ImportStorageClass::FormatNormalization, "fixture-samplers"),
        sampler_names: scope.services()?.metered_vec(
            ImportStorageClass::FormatNormalization,
            "fixture-sampler-names",
        ),
        sampler_provenance: values(&mut scope.services()?, &[])?,
        textures: scope
            .services()?
            .metered_vec(ImportStorageClass::FormatNormalization, "fixture-textures"),
        texture_provenance: values(&mut scope.services()?, &[])?,
        texture_names: scope.services()?.metered_vec(
            ImportStorageClass::FormatNormalization,
            "fixture-texture-names",
        ),
        bindings: scope
            .services()?
            .metered_vec(ImportStorageClass::FormatNormalization, "fixture-bindings"),
        materials: scope
            .services()?
            .metered_vec(ImportStorageClass::FormatNormalization, "fixture-materials"),
        image_ids: values(&mut scope.services()?, &[])?,
        sampler_ids: values(&mut scope.services()?, &[])?,
        texture_ids: values(&mut scope.services()?, &[])?,
        default_sampler: None,
        default_material: None,
        material_ids: values(&mut scope.services()?, &[])?,
    };
    let mesh = LogicalMeshStorage {
        name: owned_name(scope, Some("hand-built mesh"), "fixture-mesh-name")?,
        primitives: one(&mut scope.services()?, primitive)?,
    };
    let node_id = NodeLocalId::new(0);
    let geometry = GeometryStorage {
        logical_meshes: one(&mut scope.services()?, mesh)?,
        logical_mesh_ids: values(&mut scope.services()?, &[Some(LogicalMeshLocalId::new(0))])?,
        primitive_provenance: values(&mut scope.services()?, &[0])?,
    };
    let nodes = NodeStorage {
        roots: values(&mut scope.services()?, &[node_id])?,
        nodes: values(
            &mut scope.services()?,
            &[NodeAuthoringNode::new(
                Presence::Absent,
                AuthoredTransform::identity_trs(),
                Presence::Absent,
                if matches!(fault, FixtureFault::Light) {
                    Presence::Present(LightLocalId::new(0))
                } else {
                    Presence::Absent
                },
            )],
        )?,
        names: {
            let argument_0 = owned_name(scope, Some("手工节点"), "fixture-node-name")?;
            one(&mut scope.services()?, argument_0)
        }?,
        node_ids: values(&mut scope.services()?, &[Some(node_id)])?,
        mesh_instances: one(
            &mut scope.services()?,
            MeshInstanceStorage {
                authored: GeometryMeshInstanceAuthoring::new(node_id, LogicalMeshLocalId::new(0)),
                skin_index: None,
                source_node_index: 0,
            },
        )?,
    };
    let morphs = morph::MorphStorage {
        definitions: values(
            &mut scope.services()?,
            &[MorphTargetDefinitionAuthoring::new(
                MorphTargetIdentity::new(0),
                Presence::Absent,
            )],
        )?,
        logical_meshes: {
            let argument_0 = morph::LogicalMorphStorage {
                definitions: AuthoringRange::new(0, 1),
                weights: values(&mut scope.services()?, &[0.5])?,
            };
            one(&mut scope.services()?, argument_0)
        }?,
        primitives: one(&mut scope.services()?, 0..1)?,
        targets: one(
            &mut scope.services()?,
            morph::MorphTargetStorage { deltas: 0..1 },
        )?,
        deltas: {
            let argument_0 = morph::MorphDeltaStorage {
                semantic: MorphDeltaSemantic::Position,
                values: values(
                    &mut scope.services()?,
                    &[0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0, 3.0],
                )?,
                provenance: AccessorSources::ROOT,
            };
            one(&mut scope.services()?, argument_0)
        }?,
    };
    Ok(NormalizedGltf {
        selection_request: SourceScopeSelectionRequest::DocumentDefault,
        sources,
        appearance,
        geometry,
        cameras: CameraStorage {
            authored: scope
                .services()?
                .metered_vec(ImportStorageClass::FormatNormalization, "fixture-cameras"),
            names: scope.services()?.metered_vec(
                ImportStorageClass::FormatNormalization,
                "fixture-camera-names",
            ),
            camera_ids: values(&mut scope.services()?, &[])?,
        },
        lights: if matches!(fault, FixtureFault::Light) {
            super::super::lights::LightStorage {
                authored: one(
                    &mut scope.services()?,
                    LightDefinition::try_from_scaled_linear_srgb(
                        LightKind::point(Some(8.0)).unwrap(),
                        LocalAxes::STANDARD,
                        Color::WHITE,
                        2.0,
                    )
                    .unwrap(),
                )?,
                names: {
                    let argument_0 =
                        owned_name(scope, Some("Hand-built light"), "fixture-light-name")?;
                    one(&mut scope.services()?, argument_0)
                }?,
                light_ids: values(&mut scope.services()?, &[Some(LightLocalId::new(0))])?,
            }
        } else {
            super::super::lights::LightStorage {
                authored: values(&mut scope.services()?, &[])?,
                names: scope.services()?.metered_vec(
                    ImportStorageClass::FormatNormalization,
                    "fixture-light-names",
                ),
                light_ids: values(&mut scope.services()?, &[])?,
            }
        },
        nodes,
        skins: SkinStorage {
            skins: scope
                .services()?
                .metered_vec(ImportStorageClass::FormatNormalization, "fixture-skins"),
            skin_ids: values(&mut scope.services()?, &[])?,
        },
        instance_deformations: {
            let argument_0 = InstanceDeformationStorage {
                skin: Presence::Absent,
                weights: Some(values(&mut scope.services()?, &[0.75])?),
            };
            one(&mut scope.services()?, argument_0)
        }?,
        buffers: BufferResources {
            values: scope
                .services()?
                .metered_vec(ImportStorageClass::FormatNormalization, "fixture-buffers"),
        },
        external_images: ExternalImageResources {
            values: scope
                .services()?
                .metered_vec(ImportStorageClass::FormatNormalization, "fixture-images"),
            resource_offset: 1,
        },
        animations: animation::AnimationStorage::empty(&mut scope.services()?),
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
    let request = registry.freeze(RegistryBuildLimits::default())?.plan(
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
    Ok(request.read_asset(&io, path, ImportOperationControl::default()))
}

#[test]
fn hand_built_light_reaches_spatial_authoring_and_root_definition_provenance()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    // The light payload deliberately has no decoder meaning; this seam submits the owned model.
    let primary = br#"{"fixture":true,"nodes":[{}],"meshes":[{"primitives":[{}]}],"accessors":[{},{}],"extensions":{"KHR_lights_punctual":{"lights":[{"fixture":true}]}}}"#;
    let report = import_fixture(primary, FixtureFault::Light)??;
    let scene = report.scene();
    let light = scene.lights().next().unwrap();
    assert_eq!(scene.lights().count(), 1);
    assert_eq!(
        scene.nodes().next().unwrap().light().unwrap().id(),
        light.id()
    );
    assert_eq!(light.definition().intensity(), 2.0);
    assert_eq!(light.definition().kind().range(), Some(8.0));
    let offset = std::str::from_utf8(primary)?
        .find(r#"{"fixture":true}]}}"#)
        .unwrap();
    let expected = format!("kind=light;parent=-;index=0;offset={offset};length=16");
    assert!(scene.owner_provenance(light.metadata_owner())?.any(|node| {
        matches!(node.kind(), baozi_core::ProvenanceKindRef::Source(source)
            if matches!(source.locator(), baozi_import::SourceObjectLocatorRef::Namespaced { value, .. }
                if value == expected))
    }));
    Ok(())
}

#[test]
fn canonical_authoring_accepts_a_hand_built_model_without_a_decoder()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let report = import_fixture(FIXTURE_PRIMARY, FixtureFault::None)??;
    let scene = report.scene();
    let space = scene.space();
    assert_eq!(space.handedness, Handedness::Right);
    assert_eq!(space.up_axis, Axis::PositiveY);
    assert_eq!(space.front_axis, Axis::PositiveZ);
    assert_eq!(space.unit_scale_to_meters, Some(1.0));
    assert_eq!(space.front_face_winding, FrontFaceWinding::CounterClockwise);
    assert_eq!(space.uv_origin, UvOrigin::UpperLeft);
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
    assert!(
        primitive
            .attributes()
            .all(|attribute| attribute.tangent_conventions().is_none())
    );
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

fn inspect_source_inventory(
    scope: &mut ParseRoot<'_, '_, '_>,
    map: &GltfSourceMap<'_>,
) -> Result<()> {
    use source_profile::{DOCUMENT, OwnerSourceRun, SelectedSourceProfile, entity};
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
    let mut run = OwnerSourceRun::new(scope)?;
    let suspended = OwnerSourceRun::new(scope)?;
    let owner = ProfileOwner::Node(NodeLocalId::new(1));
    sources.link(scope, &mut run, owner, node_one)?;
    sources.complete_owner(scope, &mut run, owner)?;
    let owner = ProfileOwner::Node(NodeLocalId::new(0));
    sources.link(scope, &mut run, owner, node_zero)?;
    sources.link(scope, &mut run, owner, node_zero)?;
    sources.complete_owner(scope, &mut run, owner)?;
    sources.finish(scope, &run, &suspended)?;
    assert_eq!(sources.link_count(), 2);
    let ordinal = sources.for_owner(scope, owner)?[0];
    assert_eq!(sources.source(scope, ordinal)?.entity.id, node_zero);
    assert_ne!(
        sources.source_index(scope, 1, node_zero)?,
        sources.source_index(scope, 1, node_one)?
    );
    let mut owners = scope.services()?.metered_vec(
        ImportStorageClass::FormatNormalization,
        "fixture-source-order-owners",
    );
    let mut provenance = scope.services()?.metered_vec(
        ImportStorageClass::FormatNormalization,
        "fixture-source-order-values",
    );
    push_owner_sources(
        scope,
        &mut owners,
        &mut provenance,
        &sources,
        3,
        owner,
        [0, 1, 2],
    )?;
    let other = ProfileOwner::Node(NodeLocalId::new(1));
    push_owner_sources(
        scope,
        &mut owners,
        &mut provenance,
        &sources,
        3,
        other,
        [0, 2],
    )?;
    assert_eq!(
        provenance.as_slice(),
        [0, 1, 2, 3, 0, 2, 4].map(ProvenanceLocalId::new)
    );
    assert_eq!(owners.as_slice()[0].provenance_count, 4);
    assert_eq!(owners.as_slice()[1].provenance_offset, 4);
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
