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
fn import_consumes_only_the_core_owned_opaque_scene_plan() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for path in rust_source_files(&source_root) {
        let source = fs::read_to_string(&path).expect("read import Rust source");
        for forbidden in [
            "SceneActivationRequest",
            "SceneActivationRequestError",
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
    }
}
