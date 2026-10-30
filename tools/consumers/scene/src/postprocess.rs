use baozi::{
    AttributeData, AttributeDomain, AttributeElement, AttributeSemanticRef, Axis, FrontFaceWinding,
    Handedness, Importer, SceneSpaceField, TargetSpace, TopologyKind,
    postprocess::{
        ExistingNormals, NormalOptions, PostProcessControl, PostProcessOperation,
        PostProcessRunDisposition, PrimitiveCorner, SpaceConversionOptions, VertexAttribute,
        convert_scene_space, generate_normals, triangle_vertices, triangulate_primitive,
    },
};

pub fn verify(importer: &Importer) -> baozi::Result<()> {
    let import = importer.read_bytes(
        "textured-quad.obj",
        b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\n\
          vt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nvn 1 0 0\n\
          s 7\nf 1/1/1 2/2/1 3/3/1 4/4/1\n",
    )?;
    let (scene, import_evidence) = import.into_parts();
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    let target = primitive.id();
    assert_eq!(primitive.topology(), TopologyKind::Polygons);
    let (scene, report) = triangulate_primitive(scene, target, PostProcessControl::default())
        .expect("the independent public quad must triangulate")
        .into_parts();
    assert_eq!(report.disposition(), PostProcessRunDisposition::Changed);
    assert_eq!(
        report.operation(),
        PostProcessOperation::Triangulate { target }
    );
    assert_eq!(report.input_generation(), target.generation());
    assert_eq!(report.output_generation(), scene.generation());
    assert!(report.matches_scene(&scene));
    assert!(!import_evidence.matches_scene(&scene));
    assert_eq!(report.resources().current_live_bytes(), 0);
    assert_eq!(report.resources().active_reservations(), 0);

    let remap = report.triangulation_remap().unwrap();
    let output = scene.primitive(remap.output_target()).unwrap();
    assert_eq!(output.topology(), TopologyKind::TriangleList);
    assert_eq!(
        (
            output.vertex_count(),
            output.corner_count(),
            output.face_count()
        ),
        (4, 6, 2)
    );
    let AttributeData::DenseF32 {
        values: positions,
        lanes: 3,
    } = output.positions().unwrap().data()
    else {
        panic!("triangulation must preserve dense vertex positions");
    };
    assert_eq!(
        positions,
        &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0]
    );
    let AttributeData::DenseF32 {
        values: texcoords,
        lanes: 2,
    } = output.texcoords(0, AttributeDomain::Corner).unwrap().data()
    else {
        panic!("triangulation must preserve corner UVs");
    };
    let AttributeData::DenseF32 {
        values: normals,
        lanes: 3,
    } = output.normals(AttributeDomain::Corner).unwrap().data()
    else {
        panic!("triangulation must preserve authored normals");
    };
    let source_uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    for (index, &origin) in remap.source_corner_origins().iter().enumerate() {
        assert_eq!(
            &texcoords[index * 2..index * 2 + 2],
            &source_uvs[origin as usize]
        );
        assert_eq!(&normals[index * 3..index * 3 + 3], &[1.0, 0.0, 0.0]);
        assert_eq!(
            remap
                .corner_origin(PrimitiveCorner {
                    primitive: output.id(),
                    index: index as u32
                })
                .unwrap(),
            PrimitiveCorner {
                primitive: target,
                index: origin
            }
        );
    }
    let mut twice_area = 0.0;
    for triangle in output.indices().unwrap().chunks_exact(3) {
        let point = |index: u32| {
            let start = index as usize * 3;
            [positions[start], positions[start + 1]]
        };
        let [a, b, c] = [point(triangle[0]), point(triangle[1]), point(triangle[2])];
        let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        assert!(area > 0.0, "each triangle must preserve winding");
        twice_area += area;
    }
    assert_eq!(twice_area, 2.0);
    let expected_uvs = texcoords.to_vec();
    let normal_target = remap.output_target();
    let (scene, preserved) = generate_normals(
        scene,
        normal_target,
        NormalOptions::default(),
        PostProcessControl::default(),
    )
    .expect("authored normals are preserved by default")
    .into_parts();
    assert_eq!(
        preserved.disposition(),
        PostProcessRunDisposition::Unchanged
    );
    assert_eq!(scene.generation(), normal_target.generation());
    let options = NormalOptions {
        existing: ExistingNormals::Replace,
        ..NormalOptions::default()
    };
    let (scene, generated) =
        generate_normals(scene, normal_target, options, PostProcessControl::default())
            .expect("explicit normal replacement must publish")
            .into_parts();
    assert_eq!(generated.disposition(), PostProcessRunDisposition::Changed);
    assert!(generated.matches_scene(&scene));
    assert_eq!(generated.resources().current_live_bytes(), 0);
    assert_eq!(generated.resources().active_reservations(), 0);
    let normal_evidence = generated.normal_generation().unwrap();
    assert_eq!(normal_evidence.input_target(), normal_target);
    assert!(normal_evidence.replaced_authored_normals());
    assert!(!normal_evidence.removed_tangents());
    assert_eq!(normal_evidence.removed_metadata_entries(), 0);
    let output = scene.primitive(normal_evidence.output_target()).unwrap();
    assert_eq!(
        output.normals(AttributeDomain::Corner).unwrap().domain(),
        AttributeDomain::Corner
    );
    let AttributeData::DenseF32 { values, lanes: 3 } =
        output.normals(AttributeDomain::Corner).unwrap().data()
    else {
        panic!("generated corner normals must be dense F32");
    };
    assert!(
        values
            .chunks_exact(3)
            .all(|normal| normal == [0.0, 0.0, 1.0])
    );
    let AttributeData::DenseF32 { values, lanes: 2 } =
        output.texcoords(0, AttributeDomain::Corner).unwrap().data()
    else {
        panic!("normal generation must preserve corner UVs");
    };
    assert_eq!(values, expected_uvs);
    let source_indices = output.indices().unwrap().to_vec();
    let source_generation = scene.generation();
    let target = TargetSpace::try_new(
        Handedness::Left,
        Axis::PositiveY,
        Axis::NegativeZ,
        0.01,
        FrontFaceWinding::CounterClockwise,
    )
    .unwrap();
    let (scene, conversion) = convert_scene_space(
        scene,
        SpaceConversionOptions {
            target,
            source_assumptions: Some(TargetSpace::STANDARD),
        },
        PostProcessControl::default(),
    )
    .expect("the public static Scene must convert to an explicit space")
    .into_parts();
    let effects = conversion.space_conversion_effects().unwrap();
    assert_eq!(effects.input_generation(), source_generation);
    assert_eq!(effects.output_generation(), scene.generation());
    assert_eq!(effects.target(), target);
    assert!(effects.assumed_source_field(SceneSpaceField::UnitScaleToMeters));
    assert!(effects.reverses_triangle_winding());
    assert_eq!(conversion.resources().current_live_bytes(), 0);
    assert_eq!(conversion.resources().active_reservations(), 0);
    let output = scene.meshes().next().unwrap().primitives().next().unwrap();
    let AttributeData::DenseF32 {
        values: positions,
        lanes: 3,
    } = output.positions().unwrap().data()
    else {
        panic!("converted positions");
    };
    assert_eq!(
        positions,
        &[
            0.0, 0.0, 0.0, -100.0, 0.0, 0.0, -100.0, 100.0, 0.0, 0.0, 100.0, 0.0
        ]
    );
    let AttributeData::DenseF32 {
        values: uvs,
        lanes: 2,
    } = output.texcoords(0, AttributeDomain::Corner).unwrap().data()
    else {
        panic!("converted UVs");
    };
    let AttributeData::DenseF32 {
        values: normals,
        lanes: 3,
    } = output.normals(AttributeDomain::Corner).unwrap().data()
    else {
        panic!("converted normals");
    };
    for triangle in 0..2 {
        for (output_lane, source_lane) in [0, 2, 1].into_iter().enumerate() {
            let corner = triangle * 3 + output_lane;
            let origin = triangle * 3 + source_lane;
            assert_eq!(output.indices().unwrap()[corner], source_indices[origin]);
            assert_eq!(
                &uvs[corner * 2..corner * 2 + 2],
                &expected_uvs[origin * 2..origin * 2 + 2]
            );
            assert_eq!(&normals[corner * 3..corner * 3 + 3], &[0.0, 0.0, 1.0]);
        }
    }
    let (vertices, indices) = {
        // The application selects its concrete vertex layout and owns its fallible allocations.
        // The borrowed helper creates one common index domain for all selected fields.
        let layout = triangle_vertices(output).unwrap();
        let mut vertices = Vec::new();
        vertices.try_reserve_exact(layout.len()).unwrap();
        let mut indices = Vec::new();
        indices.try_reserve_exact(layout.len()).unwrap();
        for vertex in layout.vertices() {
            assert!(vertex.influences().is_none());
            assert_eq!(vertex.source_corner().index, vertex.index());
            vertices.push(ApplicationVertex {
                position: f32_element(
                    vertex
                        .attribute(AttributeSemanticRef::Position, AttributeDomain::Vertex)
                        .unwrap(),
                ),
                normal: f32_element(
                    vertex
                        .attribute(AttributeSemanticRef::Normal, AttributeDomain::Corner)
                        .unwrap(),
                ),
                uv: f32_element(
                    vertex
                        .attribute(AttributeSemanticRef::Texcoord(0), AttributeDomain::Corner)
                        .unwrap(),
                ),
            });
        }
        indices.extend(layout.indices());
        (vertices, indices)
    };
    let expected: Vec<_> = output
        .indices()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(corner, &vertex)| {
            let position = vertex as usize * 3;
            ApplicationVertex {
                position: positions[position..position + 3].try_into().unwrap(),
                normal: normals[corner * 3..corner * 3 + 3].try_into().unwrap(),
                uv: uvs[corner * 2..corner * 2 + 2].try_into().unwrap(),
            }
        })
        .collect();
    drop(scene);
    // Both application buffers are complete owned artifacts after all Scene borrows end.
    assert_eq!(vertices, expected);
    assert_eq!(indices, [0, 1, 2, 3, 4, 5]);
    drop(import_evidence);
    assert_eq!(remap.source_face_triangle_offsets(), &[0, 2]);
    assert_eq!(normal_evidence.input_target(), normal_target);
    assert_eq!(effects.target(), target);
    Ok(())
}

#[derive(Debug, PartialEq)]
struct ApplicationVertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
}

fn f32_element<const N: usize>(field: VertexAttribute<'_>) -> [f32; N] {
    let AttributeElement::F32(value) = field.element() else {
        panic!("the selected application field must retain F32 elements");
    };
    value
        .try_into()
        .expect("the selected field has the declared application width")
}
