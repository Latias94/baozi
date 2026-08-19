use std::fs;
use std::path::{Path, PathBuf};

use baozi_import::SceneDraft;
use baozi_import::authoring::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, DeformationAuthoringBatch,
    GeometryAuthoringBatch, LineageAuthoringBatch, MetadataAuthoringBatch, NodeAuthoringBatch,
    Presence, ProfileAuthoringBatch, SceneAuthoringBatch, SceneSpace, SpatialAuthoringBatch,
};

fn rust_source_files(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut sources = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).expect("import source directory") {
            let path = entry.expect("import source entry").path();
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
fn custom_importers_receive_one_authority_free_authoring_surface() {
    let scene_space = SceneSpace::unknown();
    let input = SceneAuthoringBatch::new(
        SpatialAuthoringBatch::new(&scene_space, &[], &[]),
        NodeAuthoringBatch::new(Presence::Present(&[]), &[]),
        AppearanceAuthoringBatch::empty(),
        GeometryAuthoringBatch::empty(),
        DeformationAuthoringBatch::empty(),
        AnimationAuthoringBatch::empty(),
        ProfileAuthoringBatch::new(
            LineageAuthoringBatch::new(&[], &[], &[], &[]),
            MetadataAuthoringBatch::empty(),
        ),
    );

    fn accepts_input(_: SceneAuthoringBatch<'_>) {}
    fn accepts_draft(_: SceneDraft) {}

    accepts_input(input);
    let _: fn(SceneDraft) = accepts_draft;
}

#[test]
fn core_scene_integration_stays_inside_the_import_infrastructure() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for path in rust_source_files(&source_root) {
        let source = fs::read_to_string(&path).expect("read import Rust source");
        for forbidden in [
            "SceneActivationRequest",
            "SceneActivationRequestError",
            "SceneActivationRequirements",
            "CanonicalActivationPolicy",
            "CanonicalActivationRequest",
            "CanonicalActivationRequestIdentity",
            "ScopedRetainedStoreSessionOutcomeOwner",
            "into_scoped_session_outcome_owner",
            "__canonical_activation_producer",
            "CanonicalActivationDescription",
            "CanonicalActivationPlanSealer",
            "CanonicalSealedActivationPlan",
            "CanonicalSuccessRequirements",
        ] {
            assert!(
                !source.contains(forbidden),
                "import source `{}` must not construct or rebind operation activation facts through `{forbidden}`",
                path.display()
            );
        }
        if source.contains("baozi_core::scene::integration") {
            let relative = path
                .strip_prefix(&source_root)
                .expect("source remains below the import crate root")
                .to_string_lossy()
                .replace('\\', "/");
            assert!(
                matches!(
                    relative.as_str(),
                    "lib.rs" | "scope.rs" | "store.rs" | "store/scene_activation.rs"
                ),
                "core Scene transaction integration escaped import infrastructure through `{relative}`"
            );
        }
    }

    let crates_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("import crate has a workspace crates directory");
    let mut feature_consumers = Vec::new();

    for entry in fs::read_dir(crates_root).expect("workspace crates directory") {
        let crate_root = entry.expect("workspace crate entry").path();
        if !crate_root.is_dir() {
            continue;
        }
        let crate_name = crate_root
            .file_name()
            .expect("workspace crate name")
            .to_string_lossy();
        let manifest = crate_root.join("Cargo.toml");
        if manifest.is_file()
            && fs::read_to_string(&manifest)
                .expect("workspace crate manifest")
                .contains("_scene-integration")
        {
            feature_consumers.push(crate_name.to_string());
        }

        if matches!(crate_name.as_ref(), "baozi-core" | "baozi-import") {
            continue;
        }
        let source_root = crate_root.join("src");
        if !source_root.is_dir() {
            continue;
        }
        for path in rust_source_files(&source_root) {
            let source = fs::read_to_string(&path).expect("workspace Rust source");
            if !source.contains("baozi_core::scene::integration") {
                continue;
            }
            let relative = path
                .strip_prefix(&source_root)
                .expect("source remains below its crate root")
                .to_string_lossy()
                .replace('\\', "/");
            panic!("core Scene transaction integration escaped into `{crate_name}/{relative}`");
        }
    }

    feature_consumers.sort();
    assert_eq!(
        feature_consumers,
        ["baozi-core", "baozi-import"],
        "only core may define and import may enable the hidden Scene integration feature"
    );
}
