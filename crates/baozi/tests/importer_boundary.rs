use baozi::{
    Animation, AnimationChannel, AnimationInterpolation, AnimationProperty, AnimationTarget,
    AnimationValues, AssetIo, AssetPath, BaoziError, Camera, CapabilityStatus, FormatCapability,
    FormatInfo, FormatMaturity, ImportFailure, ImportOptions, Importer, Light, Material,
    MaterialProperty, Mesh, MeshBinding, MeshId, MetadataMap, MetadataValue, MorphTarget, Node,
    NodeId, PrimaryProbeOperation, PrimitiveTopology, ReadSeek, ResourceLimits, Scene,
    SceneBuilder, Skin, Texture, TextureSlot, TextureSource, Vec3, VertexAttribute,
    VertexAttributeData, VertexAttributeSemantic,
};
use baozi_core::Result as CoreResult;
use baozi_import::{
    DetectionOutcome, FormatImporter, ImportContext, Probe, ReadConfidence, ReadHint,
};
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::sync::Arc;

type SceneCorruptor = fn(&mut Scene);

struct InvalidSceneImporter {
    corrupt: SceneCorruptor,
}

struct PanickingImporter;

struct GraphSceneImporter;

struct AttributeSceneImporter;

struct ManyMeshAttributeSceneImporter;

struct TopologySceneImporter;

struct StringSceneImporter;

struct FaultingAssetIo;

struct FaultingReader(Cursor<Vec<u8>>);

const LARGE_TOPOLOGY_CELL_COUNT: usize = 65_537;
const STRING_SCENE_CANONICAL_BYTES: usize = 54;

fn string_metadata() -> MetadataMap {
    MetadataMap::from([("x".to_owned(), MetadataValue::String("x".to_owned()))])
}

impl Read for FaultingReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer)
    }
}

impl Seek for FaultingReader {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        if matches!(position, SeekFrom::End(_)) {
            Err(io::Error::other("primary measure failed"))
        } else {
            self.0.seek(position)
        }
    }
}

impl AssetIo for FaultingAssetIo {
    fn open(&self, _path: &AssetPath) -> CoreResult<Box<dyn ReadSeek + Send>> {
        Ok(Box::new(FaultingReader(Cursor::new(vec![1, 2, 3]))))
    }

    fn exists(&self, _path: &AssetPath) -> bool {
        true
    }

    fn resolve(&self, _base: &AssetPath, relative: &str) -> CoreResult<AssetPath> {
        AssetPath::new(relative)
    }
}

impl FormatImporter for InvalidSceneImporter {
    fn info(&self) -> FormatInfo {
        FormatInfo::new("invalid-scene", "Invalid Scene", &["invalid"])
            .with_maturity(FormatMaturity::Experimental)
            .with_capabilities(&[(FormatCapability::Geometry, CapabilityStatus::Unknown)])
            .with_notes("facade boundary test importer")
    }

    fn can_read(&self, _probe: &Probe<'_>, _hint: &ReadHint) -> DetectionOutcome {
        DetectionOutcome::Match(ReadConfidence::Certain)
    }

    fn read(&self, _ctx: &mut ImportContext<'_>) -> CoreResult<Scene> {
        let mut scene = SceneBuilder::new().finish()?;
        (self.corrupt)(&mut scene);
        Ok(scene)
    }
}

impl FormatImporter for PanickingImporter {
    fn info(&self) -> FormatInfo {
        FormatInfo::new("panicking", "Panicking", &["panic"])
            .with_maturity(FormatMaturity::Experimental)
            .with_capabilities(&[(FormatCapability::Geometry, CapabilityStatus::Unknown)])
            .with_notes("facade panic boundary test importer")
    }

    fn can_read(&self, _probe: &Probe<'_>, _hint: &ReadHint) -> DetectionOutcome {
        DetectionOutcome::Match(ReadConfidence::Certain)
    }

    fn read(&self, _ctx: &mut ImportContext<'_>) -> CoreResult<Scene> {
        panic!("untrusted importer panic")
    }
}

impl FormatImporter for GraphSceneImporter {
    fn info(&self) -> FormatInfo {
        FormatInfo::new("graph-scene", "Graph Scene", &["graph"])
            .with_maturity(FormatMaturity::Experimental)
            .with_capabilities(&[(FormatCapability::Hierarchy, CapabilityStatus::Supported)])
    }

    fn can_read(&self, _probe: &Probe<'_>, _hint: &ReadHint) -> DetectionOutcome {
        DetectionOutcome::Match(ReadConfidence::Certain)
    }

    fn read(&self, _ctx: &mut ImportContext<'_>) -> CoreResult<Scene> {
        let mut builder = SceneBuilder::new();
        builder.add_node(Node::default());
        builder.finish()
    }
}

impl FormatImporter for AttributeSceneImporter {
    fn info(&self) -> FormatInfo {
        FormatInfo::new("attribute-scene", "Attribute Scene", &["attr"])
            .with_maturity(FormatMaturity::Experimental)
            .with_capabilities(&[(FormatCapability::Geometry, CapabilityStatus::Supported)])
    }

    fn can_read(&self, _probe: &Probe<'_>, _hint: &ReadHint) -> DetectionOutcome {
        DetectionOutcome::Match(ReadConfidence::Certain)
    }

    fn read(&self, _ctx: &mut ImportContext<'_>) -> CoreResult<Scene> {
        let mut builder = SceneBuilder::new();
        let custom_attributes = (0..2)
            .map(|index| {
                let name = format!("test:attribute_{index}");
                VertexAttribute {
                    semantic: VertexAttributeSemantic::Custom(name.clone()),
                    name,
                    data: VertexAttributeData::F32(vec![0.0; 3]),
                    metadata: Default::default(),
                }
            })
            .collect();
        let mesh = builder.add_mesh(Mesh {
            topology: PrimitiveTopology::Points,
            positions: vec![Vec3::new(0.0, 0.0, 0.0); 3],
            custom_attributes,
            ..Mesh::default()
        });
        builder.add_child_node(
            builder.root(),
            Node {
                mesh_bindings: vec![MeshBinding::new(mesh)],
                ..Node::default()
            },
        )?;
        builder.finish()
    }
}

impl FormatImporter for ManyMeshAttributeSceneImporter {
    fn info(&self) -> FormatInfo {
        FormatInfo::new(
            "many-mesh-attributes",
            "Many Mesh Attributes",
            &["manyattr"],
        )
        .with_maturity(FormatMaturity::Experimental)
        .with_capabilities(&[(FormatCapability::Geometry, CapabilityStatus::Supported)])
    }

    fn can_read(&self, _probe: &Probe<'_>, _hint: &ReadHint) -> DetectionOutcome {
        DetectionOutcome::Match(ReadConfidence::Certain)
    }

    fn read(&self, _ctx: &mut ImportContext<'_>) -> CoreResult<Scene> {
        let mut builder = SceneBuilder::new();
        for _ in 0..257 {
            builder.add_mesh(Mesh {
                topology: PrimitiveTopology::Points,
                positions: vec![Vec3::ZERO],
                ..Mesh::default()
            });
        }
        builder.finish()
    }
}

impl FormatImporter for TopologySceneImporter {
    fn info(&self) -> FormatInfo {
        FormatInfo::new("topology-scene", "Topology Scene", &["topology"])
            .with_maturity(FormatMaturity::Experimental)
            .with_capabilities(&[(FormatCapability::Geometry, CapabilityStatus::Supported)])
    }

    fn can_read(&self, _probe: &Probe<'_>, _hint: &ReadHint) -> DetectionOutcome {
        DetectionOutcome::Match(ReadConfidence::Certain)
    }

    fn read(&self, _ctx: &mut ImportContext<'_>) -> CoreResult<Scene> {
        let mut builder = SceneBuilder::new();
        builder.add_mesh(Mesh {
            topology: PrimitiveTopology::Points,
            positions: vec![Vec3::ZERO],
            indices: vec![0; LARGE_TOPOLOGY_CELL_COUNT],
            ..Mesh::default()
        });
        builder.finish()
    }
}

impl FormatImporter for StringSceneImporter {
    fn info(&self) -> FormatInfo {
        FormatInfo::new("string-scene", "String Scene", &["strings"])
            .with_maturity(FormatMaturity::Experimental)
            .with_capabilities(&[(FormatCapability::Geometry, CapabilityStatus::Supported)])
    }

    fn can_read(&self, _probe: &Probe<'_>, _hint: &ReadHint) -> DetectionOutcome {
        DetectionOutcome::Match(ReadConfidence::Certain)
    }

    fn read(&self, _ctx: &mut ImportContext<'_>) -> CoreResult<Scene> {
        let mut builder = SceneBuilder::new();
        let external_texture = builder.add_texture(Texture {
            name: Some("x".to_owned()),
            source: TextureSource::External {
                uri: "x".to_owned(),
            },
            sampler: Default::default(),
            metadata: string_metadata(),
        });
        builder.add_texture(Texture {
            name: Some("x".to_owned()),
            source: TextureSource::Embedded {
                bytes: Arc::<[u8]>::from([]),
                mime_type: Some("x".to_owned()),
            },
            sampler: Default::default(),
            metadata: string_metadata(),
        });

        let mut material = Material {
            name: Some("x".to_owned()),
            texture_slots: vec![TextureSlot {
                texture: external_texture,
                source_key: Some("x".to_owned()),
                ..TextureSlot::default()
            }],
            metadata: string_metadata(),
            ..Material::default()
        };
        material
            .properties
            .insert("p:k".to_owned(), MaterialProperty::String("x".to_owned()));
        let material = builder.add_material(material);

        let mesh = builder.add_mesh(Mesh {
            name: Some("x".to_owned()),
            topology: PrimitiveTopology::Points,
            positions: vec![Vec3::ZERO],
            material: Some(material),
            morph_targets: vec![MorphTarget {
                name: Some("x".to_owned()),
                positions: vec![Vec3::ZERO],
                metadata: string_metadata(),
                ..MorphTarget::default()
            }],
            custom_attributes: vec![VertexAttribute {
                name: "a:b".to_owned(),
                semantic: VertexAttributeSemantic::Custom("x".to_owned()),
                data: VertexAttributeData::F32(vec![0.0]),
                metadata: string_metadata(),
            }],
            metadata: string_metadata(),
            ..Mesh::default()
        });

        let root = builder.root();
        let root_node = builder.node_mut(root).expect("builder root must exist");
        root_node.name = Some("x".to_owned());
        root_node.mesh_bindings.push(MeshBinding::new(mesh));
        root_node.metadata = string_metadata();

        builder.add_animation(Animation {
            name: Some("x".to_owned()),
            channels: vec![AnimationChannel {
                target: AnimationTarget {
                    node: root,
                    property: AnimationProperty::Translation,
                },
                interpolation: AnimationInterpolation::Linear,
                times_seconds: vec![0.0],
                values: AnimationValues::Translations(vec![Vec3::ZERO]),
                metadata: string_metadata(),
            }],
            metadata: string_metadata(),
        });
        builder.add_camera(Camera {
            name: Some("x".to_owned()),
            metadata: string_metadata(),
            ..Camera::default()
        });
        builder.add_light(Light {
            name: Some("x".to_owned()),
            metadata: string_metadata(),
            ..Light::default()
        });
        builder.add_skin(Skin {
            name: Some("x".to_owned()),
            metadata: string_metadata(),
            ..Skin::default()
        });

        let mut scene = builder.finish()?;
        scene.metadata = string_metadata();
        scene.metadata.insert(
            "utf8".to_owned(),
            MetadataValue::String("\u{5305}".to_owned()),
        );
        Ok(scene)
    }
}

#[test]
fn facade_rejects_invalid_scenes_from_registered_importers() {
    let cases: [(&str, SceneCorruptor); 4] = [
        ("root node", |scene| scene.root = NodeId::new(1)),
        ("child", |scene| {
            scene.nodes[0].children.push(NodeId::new(99));
        }),
        ("mesh reference", |scene| {
            scene.nodes[0]
                .mesh_bindings
                .push(MeshBinding::new(MeshId::new(99)));
        }),
        ("unit scale", |scene| {
            scene.space.unit_scale_to_meters = Some(f32::NAN);
        }),
    ];

    for (expected_message, corrupt) in cases {
        let mut importer = Importer::empty();
        importer.register(InvalidSceneImporter { corrupt }).unwrap();

        let error = importer
            .read_bytes("untrusted.invalid", b"untrusted importer input")
            .expect_err("invalid importer output must not receive a validated report");

        assert!(
            matches!(
                &error,
                ImportFailure::Operation {
                    error: BaoziError::InvalidScene { .. },
                    evidence,
                }
                    if evidence.len() == 1
                        && evidence[0].outcome()
                            == &DetectionOutcome::Match(ReadConfidence::Certain)
            ),
            "unexpected error for {expected_message}: {error}"
        );
        assert!(
            error.to_string().contains(expected_message),
            "error did not identify {expected_message}: {error}"
        );
    }
}

#[test]
fn facade_converts_unwinding_importer_panics_to_parse_errors() {
    let mut importer = Importer::empty();
    importer.register(PanickingImporter).unwrap();

    let error = importer
        .read_bytes("untrusted.panic", b"untrusted importer input")
        .expect_err("an importer panic must become a structured error");

    assert!(matches!(
        &error,
        ImportFailure::Operation {
            error: BaoziError::Parse { .. },
            evidence,
        }
            if evidence.len() == 1
                && evidence[0].outcome()
                    == &DetectionOutcome::Match(ReadConfidence::Certain)
    ));
    assert!(error.to_string().contains("untrusted importer panic"));
}

#[test]
fn facade_preserves_primary_probe_failures_as_io_errors() {
    let source = AssetPath::new("untrusted.invalid").unwrap();

    let error = Importer::empty()
        .read_asset(&FaultingAssetIo, source)
        .expect_err("a primary probe IO failure must abort detection");

    assert!(matches!(
        error,
        ImportFailure::Detection(baozi::DetectionFailure::PrimaryFault {
            operation: PrimaryProbeOperation::Measure,
            ..
        })
    ));
    assert!(error.to_string().contains("measure"));
    assert!(error.to_string().contains("primary measure failed"));
}

#[test]
fn facade_enforces_canonical_graph_limits_for_registered_importers() {
    let mut importer = Importer::empty();
    importer.register(GraphSceneImporter).unwrap();

    let mut exact = ImportOptions::memory();
    exact.limits.max_graph_nodes = 2;
    exact.limits.max_graph_edges = 1;
    exact.limits.max_graph_depth = 2;
    exact.limits.max_graph_work = 3;
    let report = importer
        .read_bytes_with_options("scene.graph", b"graph", exact)
        .expect("exact canonical graph limits must succeed");
    assert_eq!(report.stats().graph_nodes(), 2);
    assert_eq!(report.stats().graph_edges(), 1);
    assert_eq!(report.stats().graph_work(), 3);

    for (expected_limit, configure) in [
        (
            "max_graph_nodes",
            (|limits: &mut ResourceLimits| limits.max_graph_nodes = 1) as fn(&mut ResourceLimits),
        ),
        ("max_graph_edges", |limits: &mut ResourceLimits| {
            limits.max_graph_edges = 0;
        }),
        ("max_graph_depth", |limits: &mut ResourceLimits| {
            limits.max_graph_depth = 1;
        }),
        ("max_graph_work", |limits: &mut ResourceLimits| {
            limits.max_graph_work = 2;
        }),
    ] {
        let mut options = ImportOptions::memory();
        configure(&mut options.limits);
        let error = importer
            .read_bytes_with_options("scene.graph", b"graph", options)
            .expect_err("an exceeded canonical graph limit must fail");
        assert!(matches!(
            &error,
            ImportFailure::Operation {
                error: BaoziError::LimitExceeded { limit },
                evidence,
            } if *limit == expected_limit
                && evidence.len() == 1
                && evidence[0].outcome()
                    == &DetectionOutcome::Match(ReadConfidence::Certain)
        ));
    }
}

#[test]
fn facade_enforces_aggregate_vertex_attribute_limits() {
    let mut importer = Importer::empty();
    importer.register(AttributeSceneImporter).unwrap();

    let mut exact = ImportOptions::memory();
    exact.limits.max_vertex_attribute_streams_per_mesh = 3;
    exact.limits.max_total_vertex_attribute_streams = 3;
    exact.limits.max_total_vertex_attribute_cells = 9;
    let report = importer
        .read_bytes_with_options("scene.attr", b"attributes", exact)
        .expect("exact aggregate attribute limits must succeed");
    assert_eq!(report.stats().vertex_attribute_streams(), 3);
    assert_eq!(report.stats().vertex_attribute_cells(), 9);

    for (expected_limit, configure) in [
        (
            "max_vertex_attribute_streams_per_mesh",
            (|limits: &mut ResourceLimits| limits.max_vertex_attribute_streams_per_mesh = 2)
                as fn(&mut ResourceLimits),
        ),
        (
            "max_total_vertex_attribute_streams",
            |limits: &mut ResourceLimits| {
                limits.max_total_vertex_attribute_streams = 2;
            },
        ),
        (
            "max_total_vertex_attribute_cells",
            |limits: &mut ResourceLimits| {
                limits.max_total_vertex_attribute_cells = 8;
            },
        ),
    ] {
        let mut options = ImportOptions::memory();
        configure(&mut options.limits);
        let error = importer
            .read_bytes_with_options("scene.attr", b"attributes", options)
            .expect_err("an exceeded aggregate attribute limit must fail");
        assert!(matches!(
            &error,
            ImportFailure::Operation {
                error: BaoziError::LimitExceeded { limit },
                evidence,
            } if *limit == expected_limit
                && evidence.len() == 1
                && evidence[0].outcome()
                    == &DetectionOutcome::Match(ReadConfidence::Certain)
        ));
    }
}

#[test]
fn default_attribute_limits_are_per_mesh_and_do_not_reject_many_small_meshes() {
    let mut importer = Importer::empty();
    importer.register(ManyMeshAttributeSceneImporter).unwrap();

    let report = importer
        .read_bytes("scene.manyattr", b"attributes")
        .expect("more than 256 aggregate streams across small meshes must remain valid");

    assert_eq!(report.stats().generated_meshes(), 257);
    assert_eq!(report.stats().vertex_attribute_streams(), 257);
    assert_eq!(report.stats().vertex_attribute_cells(), 257);
}

#[test]
fn facade_enforces_aggregate_topology_cell_limits_before_validation() {
    let mut importer = Importer::empty();
    importer.register(TopologySceneImporter).unwrap();

    let mut exact = ImportOptions::memory();
    exact.limits.max_total_topology_cells = LARGE_TOPOLOGY_CELL_COUNT;
    let report = importer
        .read_bytes_with_options("scene.topology", b"topology", exact)
        .expect("an exact aggregate topology limit must succeed");
    assert_eq!(report.stats().topology_cells(), LARGE_TOPOLOGY_CELL_COUNT);

    let mut exceeded = ImportOptions::memory();
    exceeded.limits.max_total_topology_cells = LARGE_TOPOLOGY_CELL_COUNT - 1;
    let error = importer
        .read_bytes_with_options("scene.topology", b"topology", exceeded)
        .expect_err("an exceeded aggregate topology limit must fail");
    assert!(matches!(
        &error,
        ImportFailure::Operation {
            error: BaoziError::LimitExceeded { limit: "max_total_topology_cells" },
            evidence,
        } if evidence.len() == 1
            && evidence[0].outcome()
                == &DetectionOutcome::Match(ReadConfidence::Certain)
    ));
}

#[test]
fn facade_enforces_and_reports_all_canonical_owned_string_bytes() {
    let mut importer = Importer::empty();
    importer.register(StringSceneImporter).unwrap();

    let mut exact = ImportOptions::memory();
    exact.limits.max_canonical_string_bytes = STRING_SCENE_CANONICAL_BYTES;
    let report = importer
        .read_bytes_with_options("scene.strings", b"strings", exact)
        .expect("an exact canonical string byte limit must succeed");
    assert_eq!(
        report.stats().canonical_string_bytes(),
        STRING_SCENE_CANONICAL_BYTES
    );

    let mut exceeded = ImportOptions::memory();
    exceeded.limits.max_canonical_string_bytes = STRING_SCENE_CANONICAL_BYTES - 1;
    let error = importer
        .read_bytes_with_options("scene.strings", b"strings", exceeded)
        .expect_err("an exceeded canonical string byte limit must fail");
    assert!(matches!(
        &error,
        ImportFailure::Operation {
            error: BaoziError::LimitExceeded { limit: "max_canonical_string_bytes" },
            evidence,
        } if evidence.len() == 1
            && evidence[0].outcome()
                == &DetectionOutcome::Match(ReadConfidence::Certain)
    ));
}

#[test]
fn postprocess_remeasures_canonical_string_limits_and_stats() {
    let mut importer = Importer::empty();
    importer.register(StringSceneImporter).unwrap();

    let mut exact = ImportOptions::memory();
    exact.limits.max_canonical_string_bytes = STRING_SCENE_CANONICAL_BYTES + 1;
    let report = importer
        .read_bytes_with_options("scene.strings", b"strings", exact)
        .expect("the imported scene must fit before postprocess");
    let report = report
        .apply_postprocess(|mut scene| {
            scene
                .metadata
                .insert("y".to_owned(), MetadataValue::Bool(true));
            Ok(scene)
        })
        .expect("postprocess output at the frozen exact limit must succeed");
    assert_eq!(
        report.stats().canonical_string_bytes(),
        STRING_SCENE_CANONICAL_BYTES + 1
    );

    let mut exceeded = ImportOptions::memory();
    exceeded.limits.max_canonical_string_bytes = STRING_SCENE_CANONICAL_BYTES;
    let report = importer
        .read_bytes_with_options("scene.strings", b"strings", exceeded)
        .expect("the imported scene must fit before postprocess");
    let error = report
        .apply_postprocess(|mut scene| {
            scene
                .metadata
                .insert("y".to_owned(), MetadataValue::Bool(true));
            Ok(scene)
        })
        .expect_err("postprocess output above the frozen string limit must fail");
    assert!(matches!(
        &error,
        ImportFailure::Operation {
            error: BaoziError::LimitExceeded { limit: "max_canonical_string_bytes" },
            evidence,
        } if evidence.len() == 1
            && evidence[0].outcome()
                == &DetectionOutcome::Match(ReadConfidence::Certain)
    ));
}

#[test]
fn report_postprocess_transition_always_revalidates_the_scene() {
    let mut importer = Importer::empty();
    importer.register(GraphSceneImporter).unwrap();
    let report = importer.read_bytes("scene.graph", b"graph").unwrap();

    let error = report
        .apply_postprocess(|mut scene| {
            scene.root = NodeId::new(99);
            Ok(scene)
        })
        .expect_err("postprocess must not produce an invalid report");

    assert!(matches!(
        &error,
        ImportFailure::Operation {
            error: BaoziError::InvalidScene { .. },
            evidence,
        }
            if evidence.len() == 1
                && evidence[0].outcome()
                    == &DetectionOutcome::Match(ReadConfidence::Certain)
    ));
}

#[test]
fn default_and_new_register_the_same_built_in_formats() {
    let default_formats = Importer::default()
        .registry()
        .formats()
        .map(|format| format.id())
        .collect::<Vec<_>>();
    let new_formats = Importer::new()
        .registry()
        .formats()
        .map(|format| format.id())
        .collect::<Vec<_>>();

    let expected: Vec<&str> = [
        #[cfg(feature = "format-stl")]
        "stl",
        #[cfg(feature = "format-obj")]
        "obj",
        #[cfg(feature = "format-ply")]
        "ply",
    ]
    .into_iter()
    .collect();

    assert_eq!(default_formats, new_formats);
    assert_eq!(new_formats, expected);
}

#[test]
fn empty_has_no_registered_formats() {
    assert_eq!(Importer::empty().registry().formats().count(), 0);
}
