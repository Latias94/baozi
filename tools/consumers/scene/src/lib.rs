//! An independent application using only the supported Baozi facade.

use baozi::{
    AttributeData, AttributeDomain, AttributeSemanticRef, Importer, SceneLookupError, TopologyKind,
};

mod custom_scene;
mod gltf_scene;
#[cfg(feature = "postprocess")]
mod postprocess;

const OBJ: &[u8] = b"o triangle\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
const STL: &[u8] = b"solid part one\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid part one\n";
const PLY: &[u8] = b"ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nproperty double temperature\nelement face 1\nproperty list uchar uint vertex_indices\nend_header\n0 0 0 16777217\n1 0 0 1e100\n0 1 0 -0\n3 0 1 2\n";

/// Runs hand-written semantic expectations for an independently supplied format set.
///
/// The expected set is deliberately not inferred from Cargo cfg or the registry itself.
pub fn verify(expected: &[&str]) -> baozi::Result<()> {
    let projection = baozi::CameraProjection::perspective(1.0, None, 0.1, None).unwrap();
    assert_eq!(projection.kind(), baozi::CameraProjectionKind::Perspective);
    assert_eq!(
        baozi::LightKind::directional().category(),
        baozi::LightKindCategory::Directional
    );
    let importer = Importer::new()?;
    let mut expected_ids: Vec<_> = expected
        .iter()
        .map(|name| format!("org.baozi.{name}"))
        .collect();
    expected_ids.sort();
    let mut actual_ids: Vec<_> = importer
        .formats()
        .map(|format| format.id().as_str().to_owned())
        .collect();
    actual_ids.sort();
    assert_eq!(actual_ids, expected_ids);

    for (name, bytes, topology) in [
        ("obj", OBJ, TopologyKind::TriangleList),
        ("stl", STL, TopologyKind::TriangleList),
        ("ply", PLY, TopologyKind::TriangleList),
    ] {
        let source = format!("triangle.{name}");
        if !expected.contains(&name) {
            assert!(
                importer.read_bytes(source, bytes).is_err(),
                "unregistered format {name} must fail"
            );
            continue;
        }
        let report = importer.read_bytes(source, bytes)?;
        let scene = report.scene();
        assert_eq!(scene.mesh_count(), 1);
        assert_eq!(scene.primitive_count(), 1);
        assert_eq!(scene.mesh_instance_count(), 1);
        assert!(scene.root_count() > 0);
        let mesh = scene.meshes().next().unwrap();
        let primitive = mesh.primitives().next().unwrap();
        assert_eq!(primitive.topology(), topology);
        assert_eq!(primitive.vertex_count(), 3);
        assert_eq!(primitive.corner_count(), 3);
        assert_eq!(primitive.face_count(), 1);
        let positions = primitive.positions().unwrap();
        assert_eq!(positions.domain(), AttributeDomain::Vertex);
        let AttributeData::DenseF32 { lanes: 3, values } = positions.data() else {
            panic!("dense XYZ expected")
        };
        assert_eq!(values, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
        assert_eq!(primitive.indices(), Some(&[0, 1, 2][..]));
        let instance = scene.mesh_instances().next().unwrap();
        assert_eq!(instance.mesh().id(), mesh.id());
        assert_eq!(
            instance.node().authored_transform().local_matrix(),
            baozi::Mat4::IDENTITY
        );
        let root = scene.roots().next().unwrap();
        let _ = root.children().collect::<Vec<_>>();
        assert_eq!(
            scene.node(instance.node().id()).unwrap().id(),
            instance.node().id()
        );
        assert_eq!(
            scene.primitive(primitive.id()).unwrap().id(),
            primitive.id()
        );
        if name == "ply" {
            let custom = primitive
                .attribute(AttributeSemanticRef::Custom("ply:temperature"))
                .unwrap();
            let AttributeData::DenseF64 { lanes: 1, values } = custom.data() else {
                panic!("F64 expected")
            };
            assert_eq!(
                values
                    .iter()
                    .map(|value| value.to_bits())
                    .collect::<Vec<_>>(),
                [
                    16_777_217.0_f64.to_bits(),
                    1e100_f64.to_bits(),
                    (-0.0_f64).to_bits()
                ]
            );
        }
        let other = importer.read_bytes(format!("other.{name}"), bytes)?;
        assert!(matches!(
            other.scene().mesh(mesh.id()),
            Err(SceneLookupError::GenerationMismatch { .. })
        ));
    }
    if expected.contains(&"obj") {
        verify_sidecars(&importer)?;
        #[cfg(feature = "postprocess")]
        postprocess::verify(&importer)?;
    }
    if expected.contains(&"gltf") {
        gltf_scene::verify_gltf(&importer)?;
    } else {
        assert!(
            importer
                .read_bytes("empty.gltf", br#"{"asset":{"version":"2.0"}}"#)
                .is_err()
        );
    }
    custom_scene::verify()?;
    Ok(())
}

struct CountingIo {
    memory: baozi::MemoryAssetIo,
    opens: std::sync::atomic::AtomicUsize,
}

impl baozi::AssetIo for CountingIo {
    fn authority_domain(&self) -> baozi::AuthorityDomain {
        baozi::AssetIo::authority_domain(&self.memory)
    }

    fn open(&self, path: &baozi::AssetPath) -> Result<baozi::OpenedAsset, baozi::AssetOpenError> {
        self.opens
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        baozi::AssetIo::open(&self.memory, path)
    }
}

fn verify_sidecars(importer: &Importer) -> baozi::Result<()> {
    use std::sync::Arc;
    let io = CountingIo {
        memory: baozi::MemoryAssetIo::new(),
        opens: std::sync::atomic::AtomicUsize::new(0),
    };
    const IMAGE: &[u8] = b"encoded-image-owned-by-the-consumer";
    for (path, bytes) in [
        ("models/paint.obj", b"mtllib materials/paint.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nusemtl paint\nf 1/1 2/2 3/3\nusemtl shared\nf 1/1 2/2 3/3\n".as_slice()),
        ("models/materials/paint.mtl", b"newmtl paint\nKd 0.25 0.5 0.75\nmap_Kd textures/paint.png\nnewmtl shared\nmap_Kd textures/paint.png\n".as_slice()),
        ("models/materials/textures/paint.png", IMAGE),
    ] {
        io.memory.try_insert(baozi::AssetPath::new(path)?, Arc::from(bytes))?;
    }
    let report = importer.read_asset(&io, baozi::AssetPath::new("models/paint.obj")?)?;
    let scene = report.scene();
    assert_eq!(scene.material_count(), 2);
    assert_eq!(scene.image_count(), 2);
    assert!(report.loss_evidence().is_empty());
    let material = scene
        .materials()
        .find(|material| material.name() == Some("paint"))
        .unwrap();
    assert_eq!(material.name(), Some("paint"));
    assert_eq!(
        material.factors().legacy().unwrap().diffuse(),
        baozi::Vec3::new(0.25, 0.5, 0.75)
    );
    assert_eq!(material.bindings().count(), 1);
    let binding = material
        .bindings()
        .find(|binding| binding.role() == baozi::TextureRole::Diffuse)
        .unwrap();
    assert_eq!(binding.role(), baozi::TextureRole::Diffuse);
    assert_eq!(binding.texture().image().encoded_bytes(), IMAGE);
    assert_eq!(
        scene
            .image(binding.texture().image().id())
            .unwrap()
            .encoded_bytes(),
        IMAGE
    );
    assert!(scene.images().all(|image| image.encoded_bytes() == IMAGE));
    assert_eq!(io.opens.load(std::sync::atomic::Ordering::Relaxed), 3);
    Ok(())
}

/// Parses the fixed test matrix's externally supplied format expectation.
pub fn expected_formats(value: &str) -> Vec<&str> {
    if value.is_empty() {
        Vec::new()
    } else {
        value.split(',').collect()
    }
}
