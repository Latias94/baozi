use baozi_core::{
    AttributeData, AttributeDomain, AttributeElement, AttributeSemanticRef,
    CustomAttributeSemantic, FormatId, FrontFaceWinding, MaterialAssignment, SceneSpace,
    TopologyKind,
};
use baozi_import::authoring::{
    AttributeSemantic, AuthoringRange, GeometryAttributeAuthoring, GeometryLogicalMeshAuthoring,
    GeometryPrimitiveAuthoring, GeometryTopologyAuthoring, Presence, StaticSceneBuilder,
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
    ExistingNormals, NormalMode, NormalOptions, NormalOrientation, PostProcessBudgets,
    PostProcessControl, TangentOptions, TangentPolicy, TrianglePreparationOptions,
    generate_normals, list_vertices, prepare_triangles, triangulate_primitive,
};

const FORMAT: &str = "dev.baozi.test.selected-geometry";
const POSITIONS: [f64; 12] = [0.0, -0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0];
const UVS: [f64; 8] = [0.0, -0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];

#[derive(Clone, Copy)]
struct Fixture {
    count: usize,
    topology: TopologyKind,
    indexed: bool,
    degenerate: bool,
    late_bad_uv: bool,
    custom: bool,
    winding: FrontFaceWinding,
    first_list: bool,
}
impl Fixture {
    fn lists(count: usize) -> Self {
        Self {
            count,
            topology: TopologyKind::TriangleList,
            indexed: true,
            degenerate: false,
            late_bad_uv: false,
            custom: true,
            winding: FrontFaceWinding::CounterClockwise,
            first_list: false,
        }
    }
}
struct Importer(Fixture);
impl FormatImporter for Importer {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor::builder(FormatId::new(FORMAT).unwrap(), "Selected geometry fixture")
            .encoding(FormatEncoding::Binary)
            .capabilities([(FormatCapability::Geometry, CapabilityStatus::Supported)])
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
        let f = self.0;
        let ids = if f.topology == TopologyKind::TriangleList {
            vec![0, 1, 2]
        } else if f.degenerate {
            vec![0, 1, 1, 3]
        } else {
            vec![0, 1, 2, 3]
        };
        let corner_indices = [0, 2];
        let corner_values = [-0.0_f64, f64::from_bits(1.0_f64.to_bits() + 1)];
        let face_values = [f64::from_bits(1), -0.0];
        let mut attributes = Vec::new();
        let mut primitives = Vec::new();
        let bad_uvs = [0.0_f64; 8];
        for i in 0..f.count {
            let source_topology = if f.first_list && i == 0 {
                TopologyKind::TriangleList
            } else {
                f.topology
            };
            let corners = if source_topology == TopologyKind::TriangleList {
                3
            } else {
                4
            };
            let faces = if matches!(
                source_topology,
                TopologyKind::TriangleList | TopologyKind::Polygons
            ) {
                1
            } else {
                2
            };
            let first = attributes.len();
            attributes.push(GeometryAttributeAuthoring::f64(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                &POSITIONS[..corners * 3],
            ));
            attributes.push(GeometryAttributeAuthoring::f64(
                AttributeSemantic::Texcoord(0),
                AttributeDomain::Vertex,
                2,
                if f.late_bad_uv && i + 1 == f.count {
                    &bad_uvs[..corners * 2]
                } else {
                    &UVS[..corners * 2]
                },
            ));
            if f.custom {
                attributes.push(GeometryAttributeAuthoring::sparse_f64(
                    AttributeSemantic::Custom(CustomAttributeSemantic::new("test:corner").unwrap()),
                    AttributeDomain::Corner,
                    1,
                    corners as u32,
                    &corner_indices,
                    &corner_values,
                    &[-0.0],
                ));
                attributes.push(GeometryAttributeAuthoring::f64(
                    AttributeSemantic::Custom(CustomAttributeSemantic::new("test:face").unwrap()),
                    AttributeDomain::Face,
                    1,
                    &face_values[..faces],
                ));
            }
            let topology = match source_topology {
                TopologyKind::TriangleStrip => GeometryTopologyAuthoring::TriangleStrip,
                TopologyKind::TriangleFan => GeometryTopologyAuthoring::TriangleFan,
                TopologyKind::Polygons => GeometryTopologyAuthoring::Polygons(&[4]),
                _ => GeometryTopologyAuthoring::TriangleList,
            };
            primitives.push(GeometryPrimitiveAuthoring::new(
                topology,
                if f.indexed {
                    Presence::Present(&ids[..corners])
                } else {
                    Presence::Absent
                },
                MaterialAssignment::Unspecified,
                AuthoringRange::new(first as u32, (attributes.len() - first) as u32),
            ));
        }
        let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(
            0,
            primitives.len() as u32,
        ))];
        let space = SceneSpace {
            front_face_winding: f.winding,
            ..SceneSpace::STANDARD_TARGET
        };
        scope.submit_static_scene(
            StaticSceneBuilder::new(Presence::Present(&[]), &[])
                .with_scene_space(&space)
                .with_geometry(&meshes, &primitives, &attributes, &[]),
        )
    }
}
fn import(f: Fixture) -> ImportReport {
    let mut registry = RegistryBuilder::new();
    registry.register(Importer(f)).unwrap();
    let registry = registry.freeze(RegistryBuildLimits::default()).unwrap();
    let request = registry
        .compile_request(
            &ImportRequest::new().with_caller_layer(ImportRequestLayer::new(
                ImportConfigOverlay::default()
                    .with_format_selection(FormatSelection::Forced(FormatId::new(FORMAT).unwrap())),
            )),
            OptionCompileLimits::default(),
        )
        .unwrap();
    import_bytes(
        &request,
        AssetPath::new("selected.fixture").unwrap(),
        b"selected",
        ImportOperationControl::default(),
    )
    .unwrap()
}
fn targets(scene: &baozi_core::ValidatedScene) -> Vec<baozi_core::PrimitiveId> {
    scene
        .meshes()
        .flat_map(|m| m.primitives().map(|p| p.id()))
        .collect()
}
fn recipe() -> TrianglePreparationOptions {
    TrianglePreparationOptions {
        normals: Some(NormalOptions {
            mode: NormalMode::Flat,
            existing: ExistingNormals::Replace,
        }),
        tangents: TangentPolicy::Generate(TangentOptions::new(
            AttributeDomain::Corner,
            0,
            AttributeDomain::Vertex,
        )),
        ..Default::default()
    }
}
fn field_bits(scene: &baozi_core::ValidatedScene) -> Vec<Vec<Vec<u64>>> {
    scene
        .meshes()
        .flat_map(|m| m.primitives())
        .map(|p| {
            p.attributes()
                .map(|a| {
                    (0..a.len())
                        .flat_map(|i| match a.element(i).unwrap() {
                            AttributeElement::F64(v) => {
                                v.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
                            }
                            AttributeElement::F32(v) => {
                                v.iter().map(|v| u64::from(v.to_bits())).collect()
                            }
                            _ => panic!("fixture float"),
                        })
                        .collect()
                })
                .collect()
        })
        .collect()
}

#[test]
fn strip_and_fan_preserve_exact_borrowed_tuple_order_sparse_domains_and_source_bits() {
    for topology in [TopologyKind::TriangleStrip, TopologyKind::TriangleFan] {
        for indexed in [false, true] {
            for degenerate in [false, true] {
                if !indexed && degenerate {
                    continue;
                }
                let (scene, evidence) = import(Fixture {
                    topology,
                    indexed,
                    degenerate,
                    ..Fixture::lists(2)
                })
                .into_parts();
                let ids = targets(&scene);
                let source = scene.primitive(ids[0]).unwrap();
                let expected: Vec<_> = list_vertices(source)
                    .unwrap()
                    .vertices()
                    .map(|v| (v.source_corner().index, v.source_vertex().index))
                    .collect();
                let before = field_bits(&scene);
                let space = scene.space();
                let (scene, report) = prepare_triangles(
                    scene,
                    &ids[..1],
                    TrianglePreparationOptions::default(),
                    PostProcessControl::default(),
                )
                .unwrap()
                .into_parts();
                assert!(!evidence.matches_scene(&scene));
                assert_eq!(scene.space(), space);
                let changed = report.geometry_preparation().unwrap().target(0).unwrap();
                assert_eq!(
                    changed.source_corner_origins().unwrap(),
                    expected.iter().map(|v| v.0).collect::<Vec<_>>()
                );
                assert_eq!(changed.source_face_triangle_offsets().unwrap(), [0, 1, 2]);
                let primitive = scene.primitive(changed.output_target()).unwrap();
                assert_eq!(
                    primitive.indices().unwrap(),
                    expected.iter().map(|v| v.1).collect::<Vec<_>>()
                );
                assert_eq!(field_bits(&scene)[1], before[1]);
                let AttributeData::DenseF64 { values, .. } = primitive.positions().unwrap().data()
                else {
                    panic!("retained width")
                };
                assert_eq!(
                    values.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                    POSITIONS.map(f64::to_bits)
                );
                let corner = primitive
                    .attributes()
                    .find(|a| a.semantic() == AttributeSemanticRef::Custom("test:corner"))
                    .unwrap();
                let AttributeData::SparseF64 {
                    indices, default, ..
                } = corner.data()
                else {
                    panic!("retained sparse shape")
                };
                assert_eq!(
                    indices,
                    expected
                        .iter()
                        .enumerate()
                        .filter_map(|(i, &(origin, _))| [0, 2]
                            .contains(&origin)
                            .then_some(i as u32))
                        .collect::<Vec<_>>()
                );
                assert_eq!(default[0].to_bits(), (-0.0_f64).to_bits());
                for (destination, &(origin, _)) in expected.iter().enumerate() {
                    let AttributeElement::F64(actual) = corner.element(destination).unwrap() else {
                        panic!()
                    };
                    assert_eq!(actual[0].to_bits(), before[0][2][origin as usize]);
                }
                let face = primitive
                    .attributes()
                    .find(|a| a.semantic() == AttributeSemanticRef::Custom("test:face"))
                    .unwrap();
                assert_eq!(face.domain(), AttributeDomain::Face);
                for (destination, &expected) in before[0][3].iter().enumerate() {
                    let AttributeElement::F64(actual) = face.element(destination).unwrap() else {
                        panic!()
                    };
                    assert_eq!(actual[0].to_bits(), expected);
                }
            }
        }
    }
}

#[test]
fn mixed_selection_reports_final_identity_and_disposition_for_the_unchanged_target() {
    let (scene, _) = import(Fixture {
        topology: TopologyKind::TriangleStrip,
        first_list: true,
        ..Fixture::lists(2)
    })
    .into_parts();
    let ids = targets(&scene);
    let before = field_bits(&scene);
    let (scene, report) = prepare_triangles(
        scene,
        &ids,
        TrianglePreparationOptions::default(),
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    let effects = report.geometry_preparation().unwrap();
    assert_eq!(effects.len(), 2);
    assert_eq!(effects.geometry_reconstruction_count(), 1);
    let unchanged = effects
        .targets()
        .find(|target| target.input_target() == ids[0])
        .unwrap();
    assert_eq!(
        unchanged.disposition(),
        baozi_postprocess::PostProcessRunDisposition::Unchanged
    );
    assert_eq!(unchanged.output_target().generation(), scene.generation());
    assert!(unchanged.source_corner_origins().is_none());
    assert!(unchanged.output_normal_attribute_index().is_none());
    assert!(unchanged.output_tangent_attribute_index().is_none());
    assert_eq!(field_bits(&scene)[0], before[0]);
    let primitive = scene.primitive(unchanged.output_target()).unwrap();
    assert_eq!(primitive.topology(), TopologyKind::TriangleList);
    assert!(
        scene
            .owner_provenance(primitive.metadata_owner())
            .unwrap()
            .all(|provenance| matches!(
                provenance.kind(),
                baozi_core::ProvenanceKindRef::Source(_)
            ))
    );
    let changed = effects
        .targets()
        .find(|target| target.input_target() == ids[1])
        .unwrap();
    assert_eq!(
        changed.disposition(),
        baozi_postprocess::PostProcessRunDisposition::Changed
    );
    assert_eq!(changed.source_corner_origins().unwrap(), [0, 1, 2, 1, 3, 2]);
    for id in ids {
        assert!(matches!(
            scene.primitive(id),
            Err(baozi_core::SceneLookupError::GenerationMismatch { .. })
        ));
    }
    drop(scene);
    assert_ne!(
        unchanged.input_target().generation(),
        unchanged.output_target().generation()
    );
    assert_eq!(effects.len(), 2);
}

#[test]
fn batch_matches_actual_single_target_lowering_normal_and_tangent_semantics() {
    for topology in [
        TopologyKind::TriangleList,
        TopologyKind::TriangleStrip,
        TopologyKind::TriangleFan,
    ] {
        let fixture = Fixture {
            topology,
            ..Fixture::lists(4)
        };
        let (batch, _) = import(fixture).into_parts();
        let (mut singles, _) = import(fixture).into_parts();
        let mut selected = targets(&batch);
        selected.reverse();
        let (batch, report) =
            prepare_triangles(batch, &selected, recipe(), PostProcessControl::default())
                .unwrap()
                .into_parts();
        assert_eq!(report.geometry_preparation().unwrap().len(), 4);
        for i in 0..4 {
            let id = targets(&singles)[i];
            singles = triangulate_primitive(singles, id, PostProcessControl::default())
                .unwrap()
                .into_parts()
                .0;
            let id = targets(&singles)[i];
            singles = generate_normals(
                singles,
                id,
                recipe().normals.unwrap(),
                recipe().tangents,
                PostProcessControl::default(),
            )
            .unwrap()
            .into_parts()
            .0;
        }
        assert_eq!(field_bits(&batch), field_bits(&singles));
        for (batch, single) in batch
            .meshes()
            .flat_map(|m| m.primitives())
            .zip(singles.meshes().flat_map(|m| m.primitives()))
        {
            assert_eq!(batch.topology(), single.topology());
            assert_eq!(batch.indices(), single.indices());
            assert_eq!(batch.corner_count(), single.corner_count());
            assert_eq!(batch.face_count(), single.face_count());
        }
    }
    let (scene, _) = import(Fixture {
        topology: TopologyKind::TriangleFan,
        ..Fixture::lists(1)
    })
    .into_parts();
    let id = targets(&scene)[0];
    let lowered = triangulate_primitive(scene, id, PostProcessControl::default()).unwrap();
    assert_eq!(
        lowered
            .report()
            .triangulation_remap()
            .unwrap()
            .source_corner_origins(),
        [1, 2, 0, 2, 3, 0]
    );
    let preparation = lowered.report().geometry_preparation().unwrap();
    assert_eq!(preparation.geometry_reconstruction_count(), 1);
    assert_eq!(
        preparation
            .target(0)
            .unwrap()
            .source_corner_origins()
            .unwrap(),
        lowered
            .report()
            .triangulation_remap()
            .unwrap()
            .source_corner_origins()
    );
}

#[test]
fn lowering_and_field_generation_publish_their_actual_provenance_kind() {
    for generate in [false, true] {
        let (scene, _) = import(Fixture {
            topology: TopologyKind::TriangleFan,
            ..Fixture::lists(2)
        })
        .into_parts();
        let ids = targets(&scene);
        let options = TrianglePreparationOptions {
            normals: generate.then_some(NormalOptions {
                mode: NormalMode::Flat,
                ..Default::default()
            }),
            ..Default::default()
        };
        let (scene, report) =
            prepare_triangles(scene, &ids, options, PostProcessControl::default())
                .unwrap()
                .into_parts();
        for target in report.geometry_preparation().unwrap().targets() {
            let primitive = scene.primitive(target.output_target()).unwrap();
            let mut attachments = scene.owner_provenance(primitive.metadata_owner()).unwrap();
            let node = attachments.next().expect("changed primitive provenance");
            assert!(attachments.next().is_none());
            let baozi_core::ProvenanceKindRef::Operation(operation) = node.kind() else {
                panic!("selected edit operation")
            };
            assert_eq!(
                operation.kind(),
                if generate {
                    baozi_core::ProvenanceOperationKindRef::Generated
                } else {
                    baozi_core::ProvenanceOperationKindRef::Split
                }
            );
        }
    }
}

#[test]
fn fixed_primitive_reconstruction_is_linear_at_32_128_512_and_publishes_once() {
    let mut previous = None;
    for count in [32, 128, 512] {
        let (scene, _) = import(Fixture::lists(count)).into_parts();
        let generation = scene.generation();
        let ids = targets(&scene);
        let (scene, report) =
            prepare_triangles(scene, &ids, recipe(), PostProcessControl::default())
                .unwrap()
                .into_parts();
        let proof = report.geometry_preparation().unwrap();
        assert_eq!(proof.len(), count);
        assert_eq!(proof.geometry_reconstruction_count(), 1);
        assert_eq!(proof.reconstructed_primitive_count(), count);
        assert_ne!(scene.generation(), generation);
        for p in proof.targets() {
            assert_eq!(p.input_target().generation(), generation);
            assert_eq!(p.output_target().generation(), scene.generation());
        }
        if let Some((old_work, old_reconstruction)) = previous {
            assert!(report.resources().consumed_work_items() <= old_work * 6);
            assert!(proof.reconstruction_work_items() <= old_reconstruction * 4 + 32);
        }
        previous = Some((
            report.resources().consumed_work_items(),
            proof.reconstruction_work_items(),
        ));
    }
}

#[test]
fn later_target_failure_and_limits_return_exact_original_generation_and_payload() {
    let (baseline, _) = import(Fixture::lists(8)).into_parts();
    let ids = targets(&baseline);
    let baseline =
        prepare_triangles(baseline, &ids, recipe(), PostProcessControl::default()).unwrap();
    let tight_storage = baseline.report().resources().peak_live_bytes() - 1;
    let tight_work = baseline.report().resources().attempted_work_items() - 1;
    drop(baseline);
    for budgets in [
        None,
        Some(PostProcessBudgets {
            work: PostProcessBudgets::default().work,
            storage_bytes: tight_storage,
        }),
        Some(PostProcessBudgets {
            work: tight_work,
            storage_bytes: PostProcessBudgets::default().storage_bytes,
        }),
    ] {
        let (scene, evidence) = import(Fixture {
            late_bad_uv: budgets.is_none(),
            ..Fixture::lists(8)
        })
        .into_parts();
        let generation = scene.generation();
        let original = field_bits(&scene);
        let ids = targets(&scene);
        let mut options = recipe();
        if let Some(b) = budgets {
            options.budgets = b;
        }
        let failure =
            prepare_triangles(scene, &ids, options, PostProcessControl::default()).unwrap_err();
        assert_eq!(failure.input().generation(), generation);
        assert_eq!(field_bits(failure.input()), original);
        assert!(evidence.matches_scene(failure.input()));
        let baozi_postprocess::PostProcessRunError::Preparation { failure } = failure.error()
        else {
            panic!("a started transaction retains its typed failure")
        };
        let resources = failure
            .resources()
            .expect("post-start rejection publishes real terminal evidence");
        assert_eq!(resources.current_live_bytes(), 0);
        assert_eq!(resources.active_reservations(), 0);
        assert_eq!(resources.committed_canonical_bytes(), 0);
        // Attempted includes the denied envelope; no work from it may complete.
        assert!(resources.attempted_work_items() >= resources.consumed_work_items());
        assert!(resources.consumed_work_items() <= options.budgets.work);
    }
}

#[test]
fn pre_cancelled_selection_preserves_the_original_scene() {
    let (scene, evidence) = import(Fixture::lists(8)).into_parts();
    let ids = targets(&scene);
    let original = field_bits(&scene);
    let token = baozi_postprocess::PostProcessCancellationToken::new();
    token.cancel();
    let failure = prepare_triangles(
        scene,
        &ids,
        recipe(),
        PostProcessControl::default().with_cancellation(token),
    )
    .unwrap_err();
    assert_eq!(field_bits(failure.input()), original);
    assert!(evidence.matches_scene(failure.input()));
}

#[test]
fn orientation_changes_normals_and_tangent_w_without_reordering_or_relabeling_scene_space() {
    for winding in [
        FrontFaceWinding::Clockwise,
        FrontFaceWinding::CounterClockwise,
        FrontFaceWinding::Unknown,
    ] {
        for assume in [
            None,
            Some(FrontFaceWinding::Clockwise),
            Some(FrontFaceWinding::CounterClockwise),
        ] {
            let (scene, _) = import(Fixture {
                winding,
                ..Fixture::lists(1)
            })
            .into_parts();
            let space = scene.space();
            let ids = targets(&scene);
            let result = prepare_triangles(
                scene,
                &ids,
                TrianglePreparationOptions {
                    orientation: NormalOrientation::SceneFrontFace {
                        assume_unknown: assume,
                    },
                    ..recipe()
                },
                PostProcessControl::default(),
            );
            if winding == FrontFaceWinding::Unknown && assume.is_none() {
                assert!(result.is_err());
                continue;
            }
            let (scene, report) = result.unwrap().into_parts();
            assert_eq!(scene.space(), space);
            let target = report
                .geometry_preparation()
                .unwrap()
                .target(0)
                .unwrap()
                .output_target();
            let p = scene.primitive(target).unwrap();
            assert_eq!(p.indices(), Some(&[0, 1, 2][..]));
            let selected = if winding == FrontFaceWinding::Unknown {
                assume.unwrap()
            } else {
                winding
            };
            let sign = if selected == FrontFaceWinding::Clockwise {
                -1.0
            } else {
                1.0
            };
            let AttributeElement::F64(normal) = p
                .normals(AttributeDomain::Corner)
                .unwrap()
                .element(0)
                .unwrap()
            else {
                panic!()
            };
            let AttributeElement::F64(tangent) = p
                .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Corner)
                .unwrap()
                .element(0)
                .unwrap()
            else {
                panic!()
            };
            assert_eq!(normal[2], sign);
            assert_eq!(tangent[3], sign);
        }
    }
}

#[test]
fn whole_scene_winding_and_reflection_before_preparation_keep_expected_frames() {
    for winding in [
        FrontFaceWinding::Clockwise,
        FrontFaceWinding::CounterClockwise,
    ] {
        for target_winding in [
            FrontFaceWinding::Clockwise,
            FrontFaceWinding::CounterClockwise,
        ] {
            for handedness in [baozi_core::Handedness::Right, baozi_core::Handedness::Left] {
                let (scene, _) = import(Fixture {
                    custom: false,
                    winding,
                    ..Fixture::lists(1)
                })
                .into_parts();
                let target = baozi_core::TargetSpace::try_new(
                    handedness,
                    baozi_core::Axis::PositiveY,
                    baozi_core::Axis::NegativeZ,
                    1.0,
                    target_winding,
                )
                .unwrap();
                let (scene, conversion) = baozi_postprocess::convert_scene_space(
                    scene,
                    baozi_postprocess::SpaceConversionOptions {
                        target,
                        source_assumptions: None,
                    },
                    PostProcessControl::default(),
                )
                .unwrap()
                .into_parts();
                let reflected = handedness == baozi_core::Handedness::Left;
                let reversed = reflected ^ (winding != target_winding);
                let indices = if reversed { [0, 2, 1] } else { [0, 1, 2] };
                let space = scene.space();
                let ids = targets(&scene);
                assert_eq!(scene.primitive(ids[0]).unwrap().indices().unwrap(), indices);
                let (scene, report) = prepare_triangles(
                    scene,
                    &ids,
                    TrianglePreparationOptions {
                        orientation: NormalOrientation::SceneFrontFace {
                            assume_unknown: None,
                        },
                        ..recipe()
                    },
                    PostProcessControl::default(),
                )
                .unwrap()
                .into_parts();
                assert_eq!(scene.space(), space);
                assert_eq!(space.front_face_winding, target_winding);
                assert_eq!(space.handedness, handedness);
                assert!(!conversion.matches_scene(&scene));
                let p = scene
                    .primitive(
                        report
                            .geometry_preparation()
                            .unwrap()
                            .target(0)
                            .unwrap()
                            .output_target(),
                    )
                    .unwrap();
                assert_eq!(p.indices().unwrap(), indices);
                let normal_sign = if winding == FrontFaceWinding::Clockwise {
                    -1.0
                } else {
                    1.0
                };
                let tangent_sign = if reflected { -1.0 } else { 1.0 };
                for corner in 0..3 {
                    let AttributeElement::F64(normal) = p
                        .normals(AttributeDomain::Corner)
                        .unwrap()
                        .element(corner)
                        .unwrap()
                    else {
                        panic!()
                    };
                    let AttributeElement::F64(tangent) = p
                        .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Corner)
                        .unwrap()
                        .element(corner)
                        .unwrap()
                    else {
                        panic!()
                    };
                    assert_eq!(normal, [0.0, 0.0, normal_sign]);
                    assert_eq!(
                        tangent,
                        [tangent_sign, 0.0, 0.0, normal_sign * tangent_sign]
                    );
                }
            }
        }
    }
}

#[test]
fn single_normal_wrapper_reports_generated_tangent_and_tangent_only_replacement_truthfully() {
    let (scene, _) = import(Fixture::lists(1)).into_parts();
    let target = targets(&scene)[0];
    let tangent = TangentOptions::new(AttributeDomain::Corner, 0, AttributeDomain::Vertex);
    let (scene, report) = generate_normals(
        scene,
        target,
        recipe().normals.unwrap(),
        TangentPolicy::Generate(tangent),
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    let normal = report.normal_generation().unwrap();
    assert!(normal.output_tangent_attribute_index().is_some());
    assert_eq!(normal.preparation().geometry_reconstruction_count(), 1);
    let target = normal.output_target();
    let before = field_bits(&scene);
    let (scene, report) = generate_normals(
        scene,
        target,
        NormalOptions::default(),
        TangentPolicy::Generate(TangentOptions {
            existing: baozi_postprocess::ExistingTangents::Replace,
            ..tangent
        }),
        PostProcessControl::default(),
    )
    .unwrap()
    .into_parts();
    assert!(report.normal_generation().is_none());
    let prepared = report.geometry_preparation().unwrap().target(0).unwrap();
    assert!(prepared.output_normal_attribute_index().is_none());
    assert!(prepared.output_tangent_attribute_index().is_some());
    assert_eq!(field_bits(&scene), before);
}
