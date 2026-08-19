use std::fs;
use std::path::{Path, PathBuf};

use baozi_core::{OrderedMap, PreparedOrderedMapCapacity};

const CANONICAL_IMPORT_BRIDGE_SOURCE: &str =
    include_str!("../src/canonical_scene/store/api/import.rs");
const SCENE_CONSTRUCTION_SOURCE: &str =
    include_str!("../src/canonical_scene/store/api/construction.rs");

fn rust_source_files(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut sources = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).expect("core source directory") {
            let path = entry.expect("core source entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                sources.push(path);
            }
        }
    }
    sources.sort();
    sources
}

#[test]
fn prepared_ordered_map_capacity_is_nameable_by_external_callers() {
    let mut map = OrderedMap::<String, u32>::new();
    let plan = map.capacity_plan(1).expect("one entry capacity plan");
    let prepared = map
        .prepare_capacity_replacement(plan)
        .expect("one entry replacement allocation");

    fn assert_public_type(_: &PreparedOrderedMapCapacity<'_, String, u32>) {}
    assert_public_type(&prepared);
}

#[test]
fn canonical_import_bridge_consumes_only_opaque_scene_plans() {
    for forbidden in [
        "__canonical_activation_producer",
        "CanonicalActivationDescription",
        "CanonicalActivationPlanSealer",
        "CanonicalSealedActivationPlan",
        "CanonicalSuccessRequirements",
        "CanonicalTableShape",
    ] {
        assert!(
            !CANONICAL_IMPORT_BRIDGE_SOURCE.contains(forbidden),
            "the import bridge must not construct generic activation facts through `{forbidden}`"
        );
    }
}

#[test]
fn scene_construction_stays_authority_free() {
    for forbidden in [
        "RetainedStoreOperation",
        "PhysicalStorageAccount",
        "StoreDimensionKey",
        "CanonicalLane",
        "OperationOwner",
    ] {
        assert!(
            !SCENE_CONSTRUCTION_SOURCE.contains(forbidden),
            "SceneConstruction must not carry operation or storage authority through `{forbidden}`"
        );
    }
}

#[cfg(feature = "_u4-import")]
#[test]
fn scene_construction_is_nameable_only_as_one_opaque_value() {
    fn accepts_construction(_: baozi_core::__canonical_import::SceneConstruction<'_>) {}
    fn accepts_error(_: baozi_core::__canonical_import::SceneConstructionStartError) {}

    let _: fn(baozi_core::__canonical_import::SceneConstruction<'_>) = accepts_construction;
    let _: fn(baozi_core::__canonical_import::SceneConstructionStartError) = accepts_error;
}

#[test]
fn generic_activation_producer_seam_stays_inside_core_construction() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for path in rust_source_files(&source_root) {
        let source = fs::read_to_string(&path).expect("read core Rust source");
        if !source.contains("__canonical_activation_producer") {
            continue;
        }
        let relative = path
            .strip_prefix(&source_root)
            .expect("core source remains below its manifest root");
        let normalized = relative.to_string_lossy().replace('\\', "/");
        assert!(
            normalized.starts_with("canonical_scene/store/construction/"),
            "trusted activation producer seam escaped the core construction boundary through `{normalized}`"
        );
    }
}
