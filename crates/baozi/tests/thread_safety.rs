use baozi::{
    AssetPath, Camera, Error, ImportFailure, ImportReport, ImportRequest, ImportRunError, Importer,
    ImporterBuilder, Light, Material, MemoryAssetIo, Mesh, Node, PostProcessFailure,
    PostProcessPipeline, PostProcessSuccess, PreparedImportRequest, ProcessedImport,
    ProcessedImportFailure, Texture, ValidatedScene,
};

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn public_scene_and_outcome_types_are_send_sync() {
    assert_send_sync::<ValidatedScene>();
    assert_send_sync::<Node>();
    assert_send_sync::<Mesh>();
    assert_send_sync::<Material>();
    assert_send_sync::<Texture>();
    assert_send_sync::<Camera>();
    assert_send_sync::<Light>();
    assert_send_sync::<ImportReport>();
    assert_send_sync::<ImportFailure>();
    assert_send_sync::<ImportRunError>();
    assert_send_sync::<PostProcessSuccess>();
    assert_send_sync::<PostProcessFailure>();
    assert_send_sync::<ProcessedImport>();
    assert_send_sync::<ProcessedImportFailure>();
    assert_send_sync::<Error>();
}

#[test]
fn frozen_import_surfaces_are_send_sync() {
    assert_send_sync::<Importer>();
    assert_send_sync::<ImporterBuilder>();
    assert_send_sync::<ImportRequest>();
    assert_send_sync::<PreparedImportRequest>();
}

#[test]
fn io_and_postprocess_contract_types_are_send_sync() {
    assert_send_sync::<AssetPath>();
    assert_send_sync::<MemoryAssetIo>();
    assert_send_sync::<PostProcessPipeline>();
}

#[cfg(feature = "native-fs-candidate")]
#[test]
fn native_filesystem_capability_types_are_send_sync() {
    assert_send_sync::<baozi::NativeDirectoryCapability>();
    assert_send_sync::<baozi::FileSystemAssetIo>();
}
