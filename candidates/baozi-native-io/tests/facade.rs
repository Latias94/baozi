use baozi::{AssetPath, AttributeData, ImportReport, Importer, TopologyKind};
use baozi_native_io_candidate::{FileSystemAssetIo, NativeDirectoryCapability};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const STL: &[u8] = b"solid facade
facet normal 0 0 1
outer loop
vertex 0 0 0
vertex 1 0 0
vertex 0 1 0
endloop
endfacet
endsolid facade
";

const OBJ: &[u8] = b"o facade
v 0 0 0
v 1 0 0
v 0 1 0
f 1 2 3
";

struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should follow the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "baozi-native-facade-{name}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("temporary native asset directory should be creatable");
        Self { path }
    }

    fn io(&self) -> FileSystemAssetIo {
        let capability = NativeDirectoryCapability::open_ambient(&self.path)
            .expect("trusted native root should open");
        FileSystemAssetIo::from_capability(capability)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn assert_triangle(report: &ImportReport) {
    let scene = report.scene();
    assert_eq!(scene.mesh_count(), 1);
    assert_eq!(scene.primitive_count(), 1);
    let mesh = scene.meshes().next().expect("one native mesh");
    let primitive = mesh.primitives().next().expect("one triangle primitive");
    assert_eq!(primitive.topology(), TopologyKind::TriangleList);
    assert_eq!(primitive.vertex_count(), 3);
    assert_eq!(primitive.face_count(), 1);
    assert_eq!(primitive.indices(), Some([0, 1, 2].as_slice()));
    let AttributeData::DenseF32 { lanes: 3, values } =
        primitive.positions().expect("native positions").data()
    else {
        panic!("expected dense F32 positions");
    };
    assert_eq!(values, [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
}

#[test]
fn facade_reads_stl_through_native_asset_io() {
    let root = TempRoot::new("stl");
    fs::write(root.path.join("facade.stl"), STL).expect("temporary STL should be writable");
    let io = root.io();
    let source = AssetPath::new("facade.stl").expect("relative STL asset path");

    let report = Importer::new()
        .expect("STL and OBJ registry should freeze")
        .read_asset(&io, source.clone())
        .expect("native STL import should succeed");

    assert_eq!(report.source(), &source);
    assert_eq!(report.format().id().as_str(), "org.baozi.stl");
    assert_triangle(&report);
    assert!(report.diagnostics().is_empty());
}

#[test]
fn facade_reads_obj_through_native_asset_io() {
    let root = TempRoot::new("obj");
    fs::write(root.path.join("facade.obj"), OBJ).expect("temporary OBJ should be writable");
    let io = root.io();
    let source = AssetPath::new("facade.obj").expect("relative OBJ asset path");

    let report = Importer::new()
        .expect("STL and OBJ registry should freeze")
        .read_asset(&io, source.clone())
        .expect("native OBJ import should succeed");

    assert_eq!(report.source(), &source);
    assert_eq!(report.format().id().as_str(), "org.baozi.obj");
    assert_triangle(&report);
    assert!(report.diagnostics().is_empty());
}

#[test]
fn facade_reads_obj_sidecars_relative_to_their_native_owner() {
    let root = TempRoot::new("obj-sidecars");
    fs::create_dir_all(root.path.join("models/materials/textures"))
        .expect("native sidecar directories should be creatable");
    fs::create_dir_all(root.path.join("models/models/materials/textures"))
        .expect("primary-relative decoy directory should be creatable");
    fs::write(
        root.path.join("models/facade.obj"),
        b"mtllib materials/facade.mtl\nusemtl red\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
    )
    .expect("native OBJ with material library should be writable");
    fs::write(
        root.path.join("models/materials/facade.mtl"),
        b"newmtl red\nKd 1 0 0\nmap_Kd textures/red.png\n",
    )
    .expect("native MTL should be writable");
    let texture = b"facade-red-texture";
    fs::write(root.path.join("models/materials/textures/red.png"), texture)
        .expect("native encoded texture should be writable");
    fs::write(
        root.path.join("models/models/materials/textures/red.png"),
        b"wrong-primary-relative-texture-payload",
    )
    .expect("native decoy texture should be writable");
    let io = root.io();
    let source = AssetPath::new("models/facade.obj").expect("relative OBJ asset path");

    let report = Importer::new()
        .expect("STL and OBJ registry should freeze")
        .read_asset(&io, source.clone())
        .expect("native OBJ sidecar import should succeed");

    assert_eq!(report.source(), &source);
    assert_eq!(report.format().id().as_str(), "org.baozi.obj");
    assert!(report.diagnostics().is_empty());
    assert_triangle(&report);
    let scene = report.scene();
    assert_eq!(scene.material_count(), 1);
    assert_eq!(scene.image_count(), 1);
    assert_eq!(scene.sampler_count(), 1);
    assert_eq!(scene.texture_count(), 1);
    let material = scene.materials().next().expect("native material");
    assert_eq!(material.bindings().len(), 1);
    let image = scene.images().next().expect("native encoded image");
    assert_eq!(image.encoded_bytes(), texture);
}

#[test]
fn native_filesystem_capability_types_are_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}

    assert_send_sync::<NativeDirectoryCapability>();
    assert_send_sync::<FileSystemAssetIo>();
}
