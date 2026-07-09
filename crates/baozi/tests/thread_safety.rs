use baozi::{
    AssetPath, AssetScope, BaoziError, Camera, Diagnostic, ImportOptions, ImportReport, Importer,
    Light, Material, MemoryAssetIo, Mesh, Node, PostProcessPipeline, Scene, Texture,
};
use baozi_import::ImporterRegistry;

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn public_scene_result_types_are_send_sync() {
    assert_send_sync::<Scene>();
    assert_send_sync::<Node>();
    assert_send_sync::<Mesh>();
    assert_send_sync::<Material>();
    assert_send_sync::<Texture>();
    assert_send_sync::<Camera>();
    assert_send_sync::<Light>();
}

#[test]
fn importer_surface_types_are_send_sync() {
    assert_send_sync::<Importer>();
    assert_send_sync::<ImporterRegistry>();
    assert_send_sync::<ImportOptions>();
    assert_send_sync::<ImportReport>();
    assert_send_sync::<Diagnostic>();
    assert_send_sync::<BaoziError>();
}

#[test]
fn io_and_postprocess_contract_types_are_send_sync() {
    assert_send_sync::<AssetPath>();
    assert_send_sync::<AssetScope>();
    assert_send_sync::<MemoryAssetIo>();
    assert_send_sync::<PostProcessPipeline>();
}

#[cfg(feature = "native-fs")]
#[test]
fn native_filesystem_io_is_send_sync() {
    assert_send_sync::<baozi::FileSystemAssetIo>();
}
