#![allow(dead_code, reason = "integration test binaries share fixture variants")]

use baozi_core::FormatId;
use baozi_format_obj::{FORMAT_ID, ObjImporter};
use baozi_import::{
    FormatSelection, ImportConfigOverlay, ImportOperationControl, ImportReport, ImportRequest,
    ImportRequestLayer, OptionCompileLimits, RegistryBuildLimits, RegistryBuilder, ValidatedScene,
    import_asset,
};
use baozi_io::{AssetPath, MemoryAssetIo};
use std::sync::Arc;

pub fn canonical_triangle_import() -> ImportReport {
    import_obj(b"o postprocess\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n")
}

pub fn canonical_quad_import() -> ImportReport {
    import_obj(b"o postprocess-quad\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\n")
}

pub fn import_obj(bytes: &[u8]) -> ImportReport {
    let source = AssetPath::new("postprocess.obj").expect("static test path should be valid");
    let io = MemoryAssetIo::new();
    io.try_insert(source.clone(), Arc::<[u8]>::from(bytes))
        .expect("test OBJ should fit the in-memory index");

    let mut registry = RegistryBuilder::new();
    registry
        .register(ObjImporter)
        .expect("OBJ importer registration should succeed");
    let registry = registry
        .freeze(RegistryBuildLimits::default())
        .expect("test registry should freeze");
    let request = ImportRequest::new().with_caller_layer(ImportRequestLayer::new(
        ImportConfigOverlay::default().with_format_selection(FormatSelection::Forced(
            FormatId::new(FORMAT_ID).expect("OBJ format id should be valid"),
        )),
    ));
    let request = registry
        .compile_request(&request, OptionCompileLimits::default())
        .expect("test request should compile");
    import_asset(&request, &io, source, ImportOperationControl::default())
        .expect("test OBJ should import")
}

pub fn canonical_triangle_scene() -> ValidatedScene {
    canonical_triangle_import().into_parts().0
}

pub fn canonical_quad_scene() -> ValidatedScene {
    canonical_quad_import().into_parts().0
}
