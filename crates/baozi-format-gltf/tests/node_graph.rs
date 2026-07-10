mod common;

use baozi_core::{BaoziError, Result};
use baozi_import::ImportOptions;
use common::{expected_error, import_assets, import_assets_result};

fn graph_document(scene_fields: &str, nodes: &str) -> Vec<u8> {
    format!(
        r#"{{
  "asset": {{ "version": "2.0", "generator": "baozi-node-graph-test" }},
  {scene_fields}
  "nodes": [{nodes}]
}}"#
    )
    .into_bytes()
}

fn scene_with_roots(roots: &str, nodes: &str) -> Vec<u8> {
    graph_document(
        &format!(r#""scene": 0, "scenes": [{{ "nodes": [{roots}] }}],"#),
        nodes,
    )
}

fn import_graph_result(
    document: Vec<u8>,
    options: ImportOptions,
) -> Result<Result<baozi_core::Scene>> {
    let (result, diagnostics) =
        import_assets_result("graph.gltf", [("graph.gltf", document)], options)?;
    assert!(diagnostics.is_empty());
    Ok(result)
}

fn assert_parse_failure(document: Vec<u8>, expected_message: &str) -> Result<()> {
    let error = expected_error(import_graph_result(document, ImportOptions::memory())?)?;
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(
        error.to_string().contains(expected_message),
        "unexpected error: {error}"
    );
    Ok(())
}

fn assert_limit_failure(
    document: Vec<u8>,
    options: ImportOptions,
    expected_limit: &'static str,
) -> Result<()> {
    let error = expected_error(import_graph_result(document, options)?)?;
    assert_eq!(
        error,
        BaoziError::LimitExceeded {
            limit: expected_limit
        }
    );
    Ok(())
}

#[test]
fn imports_valid_forest_once_without_a_declared_scene() -> Result<()> {
    let document = graph_document(
        "",
        r#"{ "name": "RootA", "children": [1] },
           { "name": "ChildA" },
           { "name": "RootB" }"#,
    );
    let (scene, diagnostics) = import_assets(
        "graph.gltf",
        [("graph.gltf", document)],
        ImportOptions::memory(),
    )?;

    assert!(diagnostics.is_empty());
    assert_eq!(scene.nodes.len(), 4);
    assert_eq!(scene.nodes[0].children.len(), 2);
    assert_eq!(scene.nodes[1].name.as_deref(), Some("RootA"));
    assert_eq!(scene.nodes[1].children, vec![baozi_core::NodeId::new(2)]);
    assert_eq!(scene.nodes[2].name.as_deref(), Some("ChildA"));
    assert_eq!(scene.nodes[3].name.as_deref(), Some("RootB"));
    Ok(())
}

#[test]
fn uses_the_first_scene_when_no_default_scene_is_declared() -> Result<()> {
    let document = graph_document(
        r#""scenes": [{ "nodes": [1] }],"#,
        r#"{ "name": "Unselected" }, { "name": "Selected" }"#,
    );
    let (scene, diagnostics) = import_assets(
        "graph.gltf",
        [("graph.gltf", document)],
        ImportOptions::memory(),
    )?;

    assert!(diagnostics.is_empty());
    assert_eq!(scene.nodes.len(), 2);
    assert_eq!(scene.nodes[1].name.as_deref(), Some("Selected"));
    Ok(())
}

#[test]
fn allows_the_same_root_in_multiple_scenes() -> Result<()> {
    let document = graph_document(
        r#""scene": 1, "scenes": [{ "nodes": [0] }, { "nodes": [0] }],"#,
        r#"{ "name": "SharedRoot" }"#,
    );
    let (scene, diagnostics) = import_assets(
        "graph.gltf",
        [("graph.gltf", document)],
        ImportOptions::memory(),
    )?;

    assert!(diagnostics.is_empty());
    assert_eq!(scene.nodes.len(), 2);
    assert_eq!(scene.nodes[1].name.as_deref(), Some("SharedRoot"));
    Ok(())
}

#[test]
fn preserves_selected_scene_root_and_child_order() -> Result<()> {
    let document = graph_document(
        r#""scene": 0, "scenes": [{ "nodes": [2, 0] }],"#,
        r#"{ "name": "RootA", "children": [1] },
           { "name": "ChildA" },
           { "name": "RootB" }"#,
    );
    let (scene, diagnostics) = import_assets(
        "graph.gltf",
        [("graph.gltf", document)],
        ImportOptions::memory(),
    )?;

    assert!(diagnostics.is_empty());
    let names = scene
        .nodes
        .iter()
        .skip(1)
        .map(|node| node.name.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(names, vec![Some("RootB"), Some("RootA"), Some("ChildA")]);
    Ok(())
}

#[test]
fn rejects_a_self_cycle() -> Result<()> {
    assert_parse_failure(
        scene_with_roots("0", r#"{ "name": "Loop", "children": [0] }"#),
        "cycle",
    )
}

#[test]
fn rejects_a_mutual_cycle() -> Result<()> {
    assert_parse_failure(
        scene_with_roots(
            "0",
            r#"{ "name": "A", "children": [1] },
               { "name": "B", "children": [0] }"#,
        ),
        "cycle",
    )
}

#[test]
fn rejects_a_shared_child() -> Result<()> {
    assert_parse_failure(
        scene_with_roots(
            "0, 1",
            r#"{ "name": "A", "children": [2] },
               { "name": "B", "children": [2] },
               { "name": "Shared" }"#,
        ),
        "multiple parents",
    )
}

#[test]
fn graph_fuzz_corpus_terminates_with_structured_failures() -> Result<()> {
    for (document, expected) in [
        (
            include_bytes!("../../../fuzz/corpus/gltf_import/node_self_cycle.gltf").as_slice(),
            "cycle",
        ),
        (
            include_bytes!("../../../fuzz/corpus/gltf_import/node_shared_child.gltf").as_slice(),
            "multiple parents",
        ),
    ] {
        assert_parse_failure(document.to_vec(), expected)?;
    }
    Ok(())
}

#[test]
fn rejects_many_roots_sharing_one_child() -> Result<()> {
    let parent_count = 32usize;
    let roots = (0..parent_count)
        .map(|index| index.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let parents = (0..parent_count)
        .map(|index| format!(r#"{{ "name": "Parent{index}", "children": [{parent_count}] }}"#))
        .collect::<Vec<_>>()
        .join(",");
    let document = scene_with_roots(&roots, &format!(r#"{parents},{{ "name": "Shared" }}"#));

    assert_parse_failure(document, "multiple parents")
}

#[test]
fn rejects_a_layered_diamond() -> Result<()> {
    assert_parse_failure(
        scene_with_roots(
            "0",
            r#"{ "name": "Root", "children": [1, 2] },
               { "name": "Left", "children": [3] },
               { "name": "Right", "children": [3] },
               { "name": "Shared" }"#,
        ),
        "multiple parents",
    )
}

#[test]
fn rejects_an_invalid_graph_before_opening_required_buffers() -> Result<()> {
    let document = br#"{
  "asset": { "version": "2.0" },
  "scene": 0,
  "scenes": [{ "nodes": [0] }],
  "nodes": [{ "children": [0] }],
  "buffers": [{ "uri": "missing.bin", "byteLength": 4 }]
}"#
    .to_vec();

    assert_parse_failure(document, "cycle")
}

#[test]
fn rejects_a_duplicate_scene_root() -> Result<()> {
    assert_parse_failure(
        scene_with_roots("0, 0", r#"{ "name": "Duplicated" }"#),
        "duplicate root",
    )
}

#[test]
fn rejects_a_scene_root_that_is_also_a_child() -> Result<()> {
    assert_parse_failure(
        scene_with_roots(
            "0, 1",
            r#"{ "name": "Root", "children": [1] },
               { "name": "Child" }"#,
        ),
        "root node 1 has a parent",
    )
}

#[test]
fn builds_a_wide_fanout_without_duplicate_nodes() -> Result<()> {
    let child_count = 64usize;
    let roots = (1..=child_count)
        .map(|index| index.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let children = (1..=child_count)
        .map(|index| format!(r#"{{ "name": "Child{index}" }}"#))
        .collect::<Vec<_>>()
        .join(",");
    let document = scene_with_roots(
        "0",
        &format!(r#"{{ "name": "Root", "children": [{roots}] }},{children}"#),
    );
    let (scene, diagnostics) = import_assets(
        "graph.gltf",
        [("graph.gltf", document)],
        ImportOptions::memory(),
    )?;

    assert!(diagnostics.is_empty());
    assert_eq!(scene.nodes.len(), child_count + 2);
    assert_eq!(scene.nodes[1].children.len(), child_count);
    Ok(())
}

#[test]
fn accepts_exact_graph_limits() -> Result<()> {
    let document = scene_with_roots(
        "0",
        r#"{ "name": "Root", "children": [1] },
           { "name": "Middle", "children": [2] },
           { "name": "Leaf" }"#,
    );
    let mut options = ImportOptions::memory();
    options.limits.max_graph_nodes = 4;
    options.limits.max_graph_edges = 3;
    options.limits.max_graph_depth = 4;
    options.limits.max_graph_work = 7;

    let scene = import_graph_result(document, options)??;
    assert_eq!(scene.nodes.len(), 4);
    Ok(())
}

#[test]
fn rejects_graph_node_limit_excess() -> Result<()> {
    let document = scene_with_roots("0", r#"{}"#);
    let mut options = ImportOptions::memory();
    options.limits.max_graph_nodes = 1;
    assert_limit_failure(document, options, "max_graph_nodes")
}

#[test]
fn rejects_graph_edge_limit_excess() -> Result<()> {
    let document = scene_with_roots("0", r#"{}"#);
    let mut options = ImportOptions::memory();
    options.limits.max_graph_edges = 0;
    assert_limit_failure(document, options, "max_graph_edges")
}

#[test]
fn rejects_graph_depth_limit_excess() -> Result<()> {
    let document = scene_with_roots("0", r#"{}"#);
    let mut options = ImportOptions::memory();
    options.limits.max_graph_depth = 1;
    assert_limit_failure(document, options, "max_graph_depth")
}

#[test]
fn rejects_graph_work_limit_excess() -> Result<()> {
    let document = graph_document("", r#"{}"#);
    let mut options = ImportOptions::memory();
    options.limits.max_graph_work = 2;
    assert_limit_failure(document, options, "max_graph_work")
}

#[test]
fn rejects_synthetic_edges_for_many_isolated_roots_before_scene_construction() -> Result<()> {
    let node_count = 64usize;
    let roots = (0..node_count)
        .map(|index| index.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let nodes = (0..node_count).map(|_| "{}").collect::<Vec<_>>().join(", ");
    let document = scene_with_roots(&roots, &nodes);
    let mut options = ImportOptions::memory();
    options.limits.max_graph_edges = 0;

    assert_limit_failure(document, options, "max_graph_edges")
}
