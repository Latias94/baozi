use super::*;

const INDICES: [u32; 6] = [3, 1, 1, 0, 2, 3];

pub(super) fn submit(
    scope: &ParseScope<'_>,
    topology: GeometryTopologyAuthoring<'static>,
    corners: u8,
    indexed: bool,
    repeated_end: bool,
) -> ParseResult<SceneDraft> {
    let positions = [
        0.0, -0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 2.0, 0.0, 0.0, 3.0, 0.0, 0.0, 2.0, 1.0, 0.0,
    ];
    let corners = u32::from(corners);
    let faces = match topology {
        GeometryTopologyAuthoring::Points => corners,
        GeometryTopologyAuthoring::LineList => corners / 2,
        GeometryTopologyAuthoring::LineStrip => corners.saturating_sub(1),
        GeometryTopologyAuthoring::LineLoop => {
            if corners < 2 {
                0
            } else {
                corners
            }
        }
        GeometryTopologyAuthoring::TriangleList => corners / 3,
        GeometryTopologyAuthoring::TriangleStrip | GeometryTopologyAuthoring::TriangleFan => {
            corners.saturating_sub(2)
        }
        GeometryTopologyAuthoring::Polygons(_) => unreachable!(),
    };
    let mut indices = INDICES;
    if repeated_end {
        indices = [0, 1, 2, 3, 4, 0];
    }
    let vertices = if indexed { 6 } else { corners as usize };
    let sparse: Vec<_> = [1, 4]
        .into_iter()
        .filter(|&corner| corner < corners)
        .collect();
    let sparse_values = [-101, -404];
    let face_values: Vec<_> = (0..faces).map(|face| f64::from(face) + 0.5).collect();
    let vertex_values: Vec<_> = (0..vertices).map(|vertex| vertex as u32 * 11).collect();
    let attributes = [
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &positions[..vertices * 3],
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::sparse_i32(
            custom("test:corner"),
            AttributeDomain::Corner,
            1,
            corners,
            &sparse,
            &sparse_values[..sparse.len()],
            &[-7],
        ),
        GeometryAttributeAuthoring::f64(
            custom("test:face"),
            AttributeDomain::Face,
            1,
            &face_values,
        ),
        GeometryAttributeAuthoring::u32(
            custom("test:vertex"),
            AttributeDomain::Vertex,
            1,
            &vertex_values,
        ),
    ];
    let primitives = [GeometryPrimitiveAuthoring::new(
        topology,
        if indexed {
            Presence::Present(&indices[..corners as usize])
        } else {
            Presence::Absent
        },
        baozi_core::MaterialAssignment::Unspecified,
        AuthoringRange::new(0, 4),
    )];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    scope.submit_static_scene(
        StaticSceneBuilder::new(Presence::Present(&[]), &[]).with_geometry(
            &meshes,
            &primitives,
            &attributes,
            &[],
        ),
    )
}

#[test]
fn all_seven_modes_have_handwritten_list_order_and_distinct_source_domains() {
    let cases: &[(
        GeometryTopologyAuthoring<'static>,
        TopologyKind,
        usize,
        &[u32],
    )] = &[
        (
            GeometryTopologyAuthoring::Points,
            TopologyKind::Points,
            1,
            &[0, 1, 2, 3, 4, 5],
        ),
        (
            GeometryTopologyAuthoring::LineList,
            TopologyKind::LineList,
            2,
            &[0, 1, 2, 3, 4, 5],
        ),
        (
            GeometryTopologyAuthoring::LineStrip,
            TopologyKind::LineList,
            2,
            &[0, 1, 1, 2, 2, 3, 3, 4, 4, 5],
        ),
        (
            GeometryTopologyAuthoring::LineLoop,
            TopologyKind::LineList,
            2,
            &[0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 0],
        ),
        (
            GeometryTopologyAuthoring::TriangleList,
            TopologyKind::TriangleList,
            3,
            &[0, 1, 2, 3, 4, 5],
        ),
        (
            GeometryTopologyAuthoring::TriangleStrip,
            TopologyKind::TriangleList,
            3,
            &[0, 1, 2, 1, 3, 2, 2, 3, 4, 3, 5, 4],
        ),
        (
            GeometryTopologyAuthoring::TriangleFan,
            TopologyKind::TriangleList,
            3,
            &[1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 5, 0],
        ),
    ];
    for &(topology, output_topology, width, expected) in cases {
        for indexed in [false, true] {
            let report = import_fixture(Fixture::Assembly {
                topology,
                corners: 6,
                indexed,
                repeated_end: false,
            });
            let scene = report.scene();
            let primitive = first_primitive(&report);
            let snapshot_limits = SceneSnapshotLimits::try_new(0, 0, 1_000_000).unwrap();
            let before = scene.capture_snapshot(snapshot_limits).unwrap();
            let allocations = allocation_counter::measure(|| {
                let layout = list_vertices(primitive).unwrap();
                assert_eq!(layout.primitive().id(), primitive.id());
                assert_eq!(layout.topology(), output_topology);
                assert_eq!(layout.len(), expected.len());
                assert!(!layout.is_empty());
                assert_eq!(layout.indices(), 0..expected.len() as u32);
                assert_eq!(layout.vertices().len(), expected.len());
                assert!(layout.vertex(expected.len()).is_none());
                assert!(layout.vertex(usize::MAX).is_none());
                for (index, &corner) in expected.iter().enumerate() {
                    let tuple = layout.vertex(index).unwrap();
                    let vertex = if indexed {
                        INDICES[corner as usize]
                    } else {
                        corner
                    };
                    let face = index / width;
                    assert_eq!(tuple.index(), index as u32);
                    assert_eq!(
                        tuple.source_corner(),
                        PrimitiveCorner {
                            primitive: primitive.id(),
                            index: corner
                        }
                    );
                    assert_eq!(
                        tuple.source_vertex(),
                        PrimitiveVertex {
                            primitive: primitive.id(),
                            index: vertex
                        }
                    );
                    assert_eq!(
                        tuple.source_face(),
                        PrimitiveFace {
                            primitive: primitive.id(),
                            index: face as u32
                        }
                    );
                    let signed = tuple
                        .attribute(
                            AttributeSemanticRef::Custom("test:corner"),
                            AttributeDomain::Corner,
                        )
                        .unwrap();
                    assert!(
                        matches!(signed.element(), AttributeElement::I32(value) if value == [match corner { 1 => -101, 4 => -404, _ => -7 }])
                    );
                    let face_field = tuple
                        .attribute(
                            AttributeSemanticRef::Custom("test:face"),
                            AttributeDomain::Face,
                        )
                        .unwrap();
                    assert!(
                        matches!(face_field.element(), AttributeElement::F64(value) if value == [face as f64 + 0.5])
                    );
                    let vertex_field = tuple
                        .attribute(
                            AttributeSemanticRef::Custom("test:vertex"),
                            AttributeDomain::Vertex,
                        )
                        .unwrap();
                    assert!(
                        matches!(vertex_field.element(), AttributeElement::U32(value) if value == [vertex * 11])
                    );
                    let original = primitive
                        .positions()
                        .unwrap()
                        .element(vertex as usize)
                        .unwrap();
                    let sampled = tuple
                        .attribute(AttributeSemanticRef::Position, AttributeDomain::Vertex)
                        .unwrap()
                        .element();
                    assert!(
                        matches!((sampled, original), (AttributeElement::F32(left), AttributeElement::F32(right)) if std::ptr::eq(left, right))
                    );
                    assert_eq!(tuple.attributes().len(), 4);
                }
                for (tuple, &corner) in layout.vertices().rev().zip(expected.iter().rev()) {
                    assert_eq!(tuple.source_corner().index, corner);
                }
                let mut ends = layout.vertices();
                assert_eq!(ends.next().unwrap().index(), 0);
                assert_eq!(ends.next_back().unwrap().index(), expected.len() as u32 - 1);
                assert_eq!(ends.len(), expected.len() - 2);
            });
            assert_eq!(allocations.count_total, 0);
            assert_eq!(allocations.bytes_total, 0);
            assert_eq!(scene.capture_snapshot(snapshot_limits).unwrap(), before);
        }
    }
}

#[test]
fn short_core_topologies_produce_no_assembled_tuples_and_keep_source_data() {
    for (topology, maximum_empty) in [
        (GeometryTopologyAuthoring::Points, 0),
        (GeometryTopologyAuthoring::LineList, 0),
        (GeometryTopologyAuthoring::TriangleList, 0),
        (GeometryTopologyAuthoring::LineStrip, 1),
        (GeometryTopologyAuthoring::LineLoop, 1),
        (GeometryTopologyAuthoring::TriangleStrip, 2),
        (GeometryTopologyAuthoring::TriangleFan, 2),
    ] {
        for corners in 0..=maximum_empty {
            for indexed in [false, true] {
                let report = import_fixture(Fixture::Assembly {
                    topology,
                    corners,
                    indexed,
                    repeated_end: false,
                });
                let primitive = first_primitive(&report);
                assert_eq!(primitive.corner_count(), u32::from(corners));
                let layout = list_vertices(primitive).unwrap();
                assert!(layout.is_empty());
                assert_eq!(layout.indices(), 0..0);
                assert_eq!(layout.vertices().len(), 0);
                assert!(layout.vertex(0).is_none());
                assert!(layout.vertices().next_back().is_none());
                assert_eq!(
                    primitive.vertex_count(),
                    if indexed { 6 } else { u32::from(corners) }
                );
            }
        }
    }
}

#[test]
fn loops_keep_two_directions_and_a_degenerate_authored_closure() {
    let report = import_fixture(Fixture::Assembly {
        topology: GeometryTopologyAuthoring::LineLoop,
        corners: 2,
        indexed: true,
        repeated_end: false,
    });
    let layout = list_vertices(first_primitive(&report)).unwrap();
    assert_eq!(
        layout
            .vertices()
            .map(|tuple| tuple.source_vertex().index)
            .collect::<Vec<_>>(),
        [3, 1, 1, 3]
    );
    let report = import_fixture(Fixture::Assembly {
        topology: GeometryTopologyAuthoring::LineLoop,
        corners: 6,
        indexed: true,
        repeated_end: true,
    });
    let layout = list_vertices(first_primitive(&report)).unwrap();
    assert_eq!(layout.len(), 12);
    assert_eq!(layout.vertex(10).unwrap().source_vertex().index, 0);
    assert_eq!(layout.vertex(11).unwrap().source_vertex().index, 0);
    assert_eq!(layout.vertex(10).unwrap().source_face().index, 5);
}

#[test]
fn list_lowering_preserves_complete_influences_and_dense_sparse_morphs() {
    for (topology, expected_corners) in [
        (
            GeometryTopologyAuthoring::TriangleStrip,
            &[0, 1, 2, 1, 3, 2, 2, 3, 4, 3, 5, 4][..],
        ),
        (
            GeometryTopologyAuthoring::TriangleFan,
            &[1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 5, 0][..],
        ),
        (
            GeometryTopologyAuthoring::LineLoop,
            &[0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 0][..],
        ),
    ] {
        let report = import_fixture(Fixture::DeformedAssembly(topology));
        let primitive = first_primitive(&report);
        let layout = list_vertices(primitive).unwrap();
        let targets: Vec<_> = primitive
            .mesh()
            .morph_target_definitions()
            .map(|target| target.id())
            .collect();
        let allocations = allocation_counter::measure(|| {
            for (tuple, &corner) in layout.vertices().zip(expected_corners) {
                let source = [0, 1, 2, 0, 2, 1][corner as usize];
                assert_eq!(tuple.source_vertex().index, source);
                let row = tuple.influences().unwrap();
                assert_eq!(row.vertex(), source);
                match source {
                    0 => {
                        assert_eq!(row.joint_indices(), &[1, 0, 1, 0, 1]);
                        assert_eq!(row.source_orders(), &[4, 3, 2, 1, 0]);
                        assert_f32(row.weights(), &[-0.0, 0.125, 0.25, 0.375, 0.25]);
                    }
                    1 => assert!(row.is_empty()),
                    2 => {
                        assert_eq!(row.joint_indices(), &[0]);
                        assert_f32(row.weights(), &[1.0]);
                    }
                    _ => unreachable!(),
                }
                assert_vec3(
                    tuple
                        .morph_delta(targets[0], MorphDeltaSemantic::Position)
                        .unwrap()
                        .unwrap(),
                    [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]][source as usize],
                );
                assert_vec3(
                    tuple
                        .morph_delta(targets[0], MorphDeltaSemantic::Normal)
                        .unwrap()
                        .unwrap(),
                    if source == 1 {
                        [2.0, -0.0, f32::from_bits(1)]
                    } else {
                        [0.0; 3]
                    },
                );
                assert!(
                    tuple
                        .morph_delta(targets[2], MorphDeltaSemantic::Position)
                        .unwrap()
                        .is_none()
                );
                assert_vec3(
                    tuple
                        .morph_delta(targets[3], MorphDeltaSemantic::Position)
                        .unwrap()
                        .unwrap(),
                    [0.0; 3],
                );
            }
        });
        assert_eq!(allocations.count_total, 0);
    }
}

#[test]
fn polygons_require_the_explicit_geometric_triangulation_operation() {
    let report = import_fixture(Fixture::Unsupported(GeometryTopologyAuthoring::Polygons(
        &[3, 3],
    )));
    assert_eq!(
        list_vertices(first_primitive(&report)).unwrap_err(),
        VertexLayoutError::UnsupportedTopology {
            topology: TopologyKind::Polygons
        }
    );
}

#[test]
fn explicit_lowering_preserves_deformation_owners_origins_and_source_identity_tables() {
    type TupleBits = (u32, Vec<u32>, Vec<u32>, Vec<u32>, Vec<Option<[u32; 3]>>);
    fn tuples(primitive: PrimitiveRef<'_>) -> Vec<TupleBits> {
        let targets: Vec<_> = primitive
            .mesh()
            .morph_target_definitions()
            .map(|t| t.id())
            .collect();
        list_vertices(primitive)
            .unwrap()
            .vertices()
            .map(|tuple| {
                let row = tuple.influences().unwrap();
                let deltas = targets
                    .iter()
                    .flat_map(|&target| {
                        [
                            MorphDeltaSemantic::Position,
                            MorphDeltaSemantic::Normal,
                            MorphDeltaSemantic::Tangent,
                        ]
                        .map(|semantic| {
                            tuple
                                .morph_delta(target, semantic)
                                .unwrap()
                                .map(|v| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()])
                        })
                    })
                    .collect();
                (
                    tuple.source_vertex().index,
                    row.joint_indices().to_vec(),
                    row.source_orders().to_vec(),
                    row.weights().iter().map(|v| v.to_bits()).collect(),
                    deltas,
                )
            })
            .collect()
    }
    for topology in [
        GeometryTopologyAuthoring::TriangleStrip,
        GeometryTopologyAuthoring::TriangleFan,
    ] {
        let (scene, import) = import_fixture(Fixture::DeformedAssembly(topology)).into_parts();
        let source = scene.meshes().next().unwrap().primitives().next().unwrap();
        let target = source.id();
        let expected = tuples(source);
        let origins: Vec<_> = list_vertices(source)
            .unwrap()
            .vertices()
            .map(|v| v.source_corner().index)
            .collect();
        let before = scene
            .capture_snapshot(SceneSnapshotLimits::try_new(32 * 1024, 128, 1_000_000).unwrap())
            .unwrap();
        let (scene, report) = baozi_postprocess::prepare_triangles(
            scene,
            &[target],
            baozi_postprocess::TrianglePreparationOptions::default(),
            baozi_postprocess::PostProcessControl::default(),
        )
        .unwrap()
        .into_parts();
        assert!(!import.matches_scene(&scene));
        let target = report.geometry_preparation().unwrap().target(0).unwrap();
        assert_eq!(target.source_corner_origins().unwrap(), origins);
        assert_eq!(
            tuples(scene.primitive(target.output_target()).unwrap()),
            expected
        );
        let after = scene
            .capture_snapshot(SceneSnapshotLimits::try_new(32 * 1024, 128, 1_000_000).unwrap())
            .unwrap();
        assert_eq!(before.profile_source_count(), after.profile_source_count());
        assert_eq!(
            before.profile_resource_count(),
            after.profile_resource_count()
        );
        assert_eq!(
            before.profile_metadata_entry_count(),
            after.profile_metadata_entry_count()
        );
        drop(scene);
        drop(import);
        assert_eq!(target.source_corner_origins().unwrap(), origins);
    }
}
