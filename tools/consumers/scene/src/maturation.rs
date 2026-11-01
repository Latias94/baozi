//! Fixed public-interface regressions also executed from unchanged Cargo archives.

use baozi::{
    AttributeData, Importer, MaterialAssignment, RelationsQueryLimits, SpatialQueryLimits,
};

pub(crate) fn verify(
    importer: &Importer,
    expected: &[&str],
) -> Result<(), Box<dyn std::error::Error>> {
    if expected.contains(&"ply") {
        wide_geometry(importer)?;
    }
    if expected.contains(&"gltf") {
        material_and_relations(importer)?;
    }
    #[cfg(feature = "postprocess")]
    if expected.contains(&"obj") {
        selected_geometry(importer)?;
    }
    Ok(())
}

fn wide_geometry(importer: &Importer) -> Result<(), Box<dyn std::error::Error>> {
    let report = importer.read_bytes("wide.ply", b"ply\nformat ascii 1.0\nelement vertex 3\nproperty double x\nproperty double y\nproperty double z\nelement face 1\nproperty list uchar uint vertex_indices\nend_header\n16777216 0 -0\n16777217 0 0\n16777216 1 0\n3 0 1 2\n")?;
    let scene = report.scene();
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    let AttributeData::DenseF64 { lanes: 3, values } = primitive.positions().unwrap().data() else {
        panic!("double Position must remain F64");
    };
    assert_eq!(values[0], 16_777_216.0);
    assert_eq!(values[3], 16_777_217.0);
    assert_eq!(values[2].to_bits(), (-0.0_f64).to_bits());
    let copied = values.to_vec();
    let limits = SpatialQueryLimits::new(100_000, 1_048_576);
    let transforms = scene.base_pose_world_transforms(limits, None)?;
    let bounds = transforms.scene_bounds(scene, limits, None)?.unwrap();
    assert_eq!(bounds.min, [16_777_216.0, 0.0, 0.0]);
    assert_eq!(bounds.max, [16_777_217.0, 1.0, 0.0]);
    drop(report);
    assert_eq!(copied[3] - copied[0], 1.0);
    assert_eq!(copied[2].to_bits(), (-0.0_f64).to_bits());
    Ok(())
}

fn material_and_relations(importer: &Importer) -> Result<(), Box<dyn std::error::Error>> {
    let report = importer.read_bytes("defaults.gltf", br#"{
        "asset":{"version":"2.0"}, "scene":0, "scenes":[{"nodes":[0]}],
        "nodes":[{"children":[1,2]},{"mesh":0},{"mesh":0}], "materials":[{}],
        "buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],
        "bufferViews":[{"buffer":0,"byteLength":36}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0},"material":0},{"attributes":{"POSITION":0}}]}]
    }"#)?;
    let scene = report.scene();
    assert_eq!(scene.material_count(), 2);
    let mut primitives = scene.meshes().next().unwrap().primitives();
    let explicit = primitives.next().unwrap();
    let implicit = primitives.next().unwrap();
    assert!(matches!(
        explicit.material_assignment(),
        MaterialAssignment::Authored(_)
    ));
    assert!(matches!(
        implicit.material_assignment(),
        MaterialAssignment::ImplicitDefault(_)
    ));
    assert!(implicit.authored_material().is_none());
    assert_eq!(
        explicit.effective_material().unwrap().factors(),
        implicit.effective_material().unwrap().factors()
    );
    let generated = scene
        .owner_provenance(implicit.effective_material().unwrap().metadata_owner())?
        .next()
        .unwrap();
    assert!(
        matches!(generated.kind(), baozi::ProvenanceKindRef::Operation(operation)
        if operation.kind() == baozi::ProvenanceOperationKindRef::Generated)
    );

    let relations = scene.relations(RelationsQueryLimits::new(100_000, 1_048_576), None)?;
    let root = scene.roots().next().unwrap().id();
    assert_eq!(relations.parent(root)?, None);
    let placements: Vec<_> = scene
        .mesh_instances()
        .map(|instance| (instance.node().id(), instance.id()))
        .collect();
    assert_eq!(placements.len(), 2);
    drop(primitives);
    drop(report);
    // The query owns only its reverse index, and retains its original generation after teardown.
    for (node, instance) in placements {
        assert_eq!(relations.parent(node)?, Some(root));
        assert_eq!(relations.instances(node)?.collect::<Vec<_>>(), [instance]);
    }
    Ok(())
}

#[cfg(feature = "postprocess")]
fn selected_geometry(importer: &Importer) -> Result<(), Box<dyn std::error::Error>> {
    use baozi::{
        AttributeDomain, TopologyKind,
        postprocess::{
            NormalOptions, PostProcessBudgets, PostProcessControl, TrianglePreparationOptions,
            prepare_triangles,
        },
    };
    let report = importer.read_bytes("batch.obj", b"o first\nv 0 0 0\nv 2 0 0\nv 2 1 0\nv 0 1 0\nf 1 2 3 4\no second\nv 0 0 1\nv 2 0 1\nv 2 1 1\nv 0 1 1\nf 5 6 7 8\n")?;
    let (scene, import_evidence) = report.into_parts();
    let targets: Vec<_> = scene
        .meshes()
        .flat_map(|mesh| mesh.primitives())
        .map(|p| p.id())
        .collect();
    assert_eq!(targets.len(), 2);
    let (scene, report) = prepare_triangles(
        scene,
        &targets,
        TrianglePreparationOptions {
            normals: Some(NormalOptions::default()),
            budgets: PostProcessBudgets {
                work: 1_000_000,
                storage_bytes: 16 * 1024 * 1024,
            },
            ..TrianglePreparationOptions::default()
        },
        PostProcessControl::default(),
    )?
    .into_parts();
    assert!(!import_evidence.matches_scene(&scene));
    assert!(report.matches_scene(&scene));
    let preparation = report.geometry_preparation().unwrap();
    assert_eq!(preparation.geometry_reconstruction_count(), 1);
    assert_eq!(preparation.len(), 2);
    assert_eq!(report.resources().current_live_bytes(), 0);
    assert_eq!(report.resources().active_reservations(), 0);
    for target in targets {
        assert!(matches!(
            scene.primitive(target),
            Err(baozi::SceneLookupError::GenerationMismatch { .. })
        ));
    }
    for primitive in scene.meshes().flat_map(|mesh| mesh.primitives()) {
        assert_eq!(primitive.topology(), TopologyKind::TriangleList);
        assert_eq!(primitive.face_count(), 2);
        let AttributeData::DenseF32 { lanes: 3, values } =
            primitive.normals(AttributeDomain::Corner).unwrap().data()
        else {
            panic!("generated normals");
        };
        assert!(
            values
                .chunks_exact(3)
                .all(|normal| normal == [0.0, 0.0, 1.0])
        );
    }
    Ok(())
}
