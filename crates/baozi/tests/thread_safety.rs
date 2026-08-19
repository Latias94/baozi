use baozi::{
    AssetPath, Error, ImmutableMemoryAssetIo, ImportFailure, ImportReport, ImportRequest,
    ImportRunError, Importer, ImporterBuilder, MemoryAssetIo, PreparedImportRequest, SceneSnapshot,
    SceneSnapshotError, SceneSnapshotLimits, ValidatedScene,
};

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn public_scene_and_outcome_types_are_send_sync() {
    assert_send_sync::<ValidatedScene>();
    assert_send_sync::<SceneSnapshot>();
    assert_send_sync::<SceneSnapshotLimits>();
    assert_send_sync::<SceneSnapshotError>();
    assert_send_sync::<ImportReport>();
    assert_send_sync::<ImportFailure>();
    assert_send_sync::<ImportRunError>();
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
fn io_contract_types_are_send_sync() {
    assert_send_sync::<AssetPath>();
    assert_send_sync::<ImmutableMemoryAssetIo>();
    assert_send_sync::<MemoryAssetIo>();
}

#[cfg(feature = "native-fs-candidate")]
#[test]
fn native_filesystem_capability_types_are_send_sync() {
    assert_send_sync::<baozi::NativeDirectoryCapability>();
    assert_send_sync::<baozi::FileSystemAssetIo>();
}
