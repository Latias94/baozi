use std::{
    fs,
    path::{Path, PathBuf},
};

const U4_PRIVATE_MODULES: [&str; 19] = [
    "animation",
    "appearance",
    "attribute",
    "canonical_edit",
    "canonical_scene",
    "canonical_validation",
    "deform",
    "derived",
    "derived_input",
    "entity",
    "extension",
    "geometry",
    "identity",
    "image",
    "namespaced",
    "provenance",
    "remap",
    "scene_space",
    "shared_storage",
];

#[test]
fn u4_private_canonical_dead_code_allow_is_scoped_and_temporary() {
    // U4 must delete this gate when it deletes the legacy-only collection enum.
    let _legacy_cutover_guard: Option<baozi_core::SceneCollection> = None;

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("baozi-core must remain under the workspace crates directory");
    let core_lib = manifest_dir.join("src/lib.rs");
    let core_lib_source =
        fs::read_to_string(&core_lib).expect("baozi-core library source must be readable");
    let marker = ["U4-private-", "canonical-rail"].concat();
    let expected_attribute =
        format!("#[allow(dead_code, reason = \"{marker}: remove at U4 cutover\")]");

    let mut attributed_modules = Vec::new();
    let mut lines = core_lib_source.lines().enumerate();
    while let Some((line_index, line)) = lines.next() {
        if !line.contains(&marker) {
            continue;
        }

        assert_eq!(line.trim(), expected_attribute);
        let (_, declaration) = lines
            .find(|(_, candidate)| !candidate.trim().is_empty())
            .unwrap_or_else(|| {
                panic!(
                    "missing module declaration after lib.rs line {}",
                    line_index + 1
                )
            });
        let declaration = declaration.trim();
        let module = declaration
            .strip_prefix("mod ")
            .and_then(|value| value.strip_suffix(';'))
            .unwrap_or_else(|| {
                panic!(
                    "the U4 allow at lib.rs line {} must annotate one private module declaration",
                    line_index + 1
                )
            });
        attributed_modules.push(module.to_owned());
    }

    let expected_modules = U4_PRIVATE_MODULES.map(str::to_owned);
    assert_eq!(
        attributed_modules, expected_modules,
        "the temporary allow must cover exactly the measured U4-private module set"
    );
    let mut production_sources = Vec::new();
    for entry in fs::read_dir(workspace_root.join("crates"))
        .expect("workspace crates directory must be readable")
    {
        let crate_dir = entry
            .expect("crate directory entry must be readable")
            .path();
        let source_dir = crate_dir.join("src");
        if source_dir.is_dir() {
            collect_rust_sources(&source_dir, &mut production_sources);
        }
    }

    for source_path in production_sources {
        if source_path == core_lib {
            continue;
        }
        let source = fs::read_to_string(&source_path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", source_path.display()));
        assert!(
            !source.contains(&marker),
            "the U4 marker must not appear in production source {}",
            source_path.display()
        );
    }
}

fn collect_rust_sources(directory: &Path, sources: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()))
    {
        let path = entry
            .expect("source directory entry must be readable")
            .path();
        if path.is_dir() {
            collect_rust_sources(&path, sources);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
}
