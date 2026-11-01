#![cfg(all(feature = "format-gltf", feature = "postprocess"))]

use baozi::{
    Importer, MaterialAssignment, MetadataOwnerRef, ProvenanceKindRef, ProvenanceLookupError,
    ProvenanceOperationKindRef, SceneLookupError, ValidatedScene,
    postprocess::{
        NormalOptions, PostProcessControl, TrianglePreparationOptions, prepare_triangles,
    },
};

const SOURCE: &[u8] = br#"{
    "asset":{"version":"2.0"}, "scene":0, "scenes":[{"nodes":[0]}],
    "nodes":[{"mesh":0}], "materials":[{}],
    "buffers":[{"byteLength":36,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],
    "bufferViews":[{"buffer":0,"byteLength":36}],
    "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],
    "meshes":[{"primitives":[{"attributes":{"POSITION":0},"material":0},{"attributes":{"POSITION":0}}]}]
}"#;

fn default_material(scene: &ValidatedScene) -> baozi::MaterialRef<'_> {
    let mut primitives = scene.meshes().next().unwrap().primitives();
    assert!(matches!(
        primitives.next().unwrap().material_assignment(),
        MaterialAssignment::Authored(_)
    ));
    let primitive = primitives.next().unwrap();
    assert!(primitive.authored_material().is_none());
    let MaterialAssignment::ImplicitDefault(material) = primitive.material_assignment() else {
        panic!("source absence must remain distinguishable after an edit");
    };
    assert_eq!(primitive.effective_material().unwrap().id(), material.id());
    material
}

#[test]
fn selected_edit_preserves_implicit_assignment_and_generated_material_provenance() {
    let report = Importer::new()
        .unwrap()
        .read_bytes("materials.gltf", SOURCE)
        .unwrap();
    let (scene, import_evidence) = report.into_parts();
    assert_eq!(scene.material_count(), 2);
    let material = default_material(&scene);
    let input_material = material.id();
    let input_owner = material.metadata_owner();
    let provenance = scene.owner_provenance(input_owner).unwrap().next().unwrap();
    let input_provenance = provenance.id();
    let ProvenanceKindRef::Operation(operation) = provenance.kind() else {
        panic!("generated default");
    };
    assert_eq!(operation.kind(), ProvenanceOperationKindRef::Generated);
    let algorithm = operation.options_digest().0;
    let digest = operation.options_digest().1.to_vec();
    let mut targets = Vec::new();
    for primitive in scene.meshes().next().unwrap().primitives() {
        targets.push(primitive.id());
    }
    let output = prepare_triangles(
        scene,
        &targets,
        TrianglePreparationOptions {
            normals: Some(NormalOptions::default()),
            ..TrianglePreparationOptions::default()
        },
        PostProcessControl::default(),
    )
    .unwrap();
    let (scene, report) = output.into_parts();
    assert!(report.matches_scene(&scene));
    assert!(!import_evidence.matches_scene(&scene));
    assert_eq!(
        report
            .geometry_preparation()
            .unwrap()
            .geometry_reconstruction_count(),
        1
    );
    assert!(matches!(
        scene.material(input_material),
        Err(SceneLookupError::GenerationMismatch { .. })
    ));
    assert!(matches!(
        scene.owner_provenance(input_owner),
        Err(ProvenanceLookupError::GenerationMismatch { .. })
    ));
    assert!(matches!(
        scene.provenance(input_provenance),
        Err(ProvenanceLookupError::GenerationMismatch { .. })
    ));
    let material = default_material(&scene);
    assert_ne!(material.id(), input_material);
    assert!(
        !scene
            .metadata_entries()
            .any(|entry| entry.owner() == MetadataOwnerRef::Entity(material.metadata_owner()))
    );
    let generated = scene
        .owner_provenance(material.metadata_owner())
        .unwrap()
        .next()
        .unwrap();
    let ProvenanceKindRef::Operation(operation) = generated.kind() else {
        panic!("generated provenance survives");
    };
    assert_eq!(operation.kind(), ProvenanceOperationKindRef::Generated);
    assert_eq!(operation.options_digest().0, algorithm);
    assert_eq!(operation.options_digest().1, digest.as_slice());
    assert_eq!(
        operation.evidence().next().unwrap().code(),
        "org.baozi.gltf:implicit-material-v1"
    );
    assert_eq!(generated.parents().len(), 1);
    assert!(matches!(
        generated.parents().next().unwrap().kind(),
        ProvenanceKindRef::Source(_)
    ));
}
