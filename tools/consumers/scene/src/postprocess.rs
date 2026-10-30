use baozi::{
    AttributeData, Importer, TopologyKind,
    postprocess::{
        PostProcessControl, PostProcessOperation, PostProcessRunDisposition, PrimitiveCorner,
        triangulate_primitive,
    },
};

pub fn verify(importer: &Importer) -> baozi::Result<()> {
    let import = importer.read_bytes(
        "textured-quad.obj",
        b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\n\
          vt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nvn 0 0 1\n\
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
    } = output.texcoords(0).unwrap().data()
    else {
        panic!("triangulation must preserve corner UVs");
    };
    let AttributeData::DenseF32 {
        values: normals,
        lanes: 3,
    } = output.normals().unwrap().data()
    else {
        panic!("triangulation must preserve authored normals");
    };
    let source_uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    for (index, &origin) in remap.source_corner_origins().iter().enumerate() {
        assert_eq!(
            &texcoords[index * 2..index * 2 + 2],
            &source_uvs[origin as usize]
        );
        assert_eq!(&normals[index * 3..index * 3 + 3], &[0.0, 0.0, 1.0]);
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
    drop(scene);
    drop(import_evidence);
    assert_eq!(remap.source_face_triangle_offsets(), &[0, 2]);
    Ok(())
}
