mod common;

use baozi_format_gltf::GltfFailureCode;
use baozi_import::{
    ImportCause, ImportDimension, ImportFailure, ImportReport, ImportResult, ScopeAbortCause,
    SourceScopeSelectionOutcome,
};
use baozi_io::AssetPath;
use baozi_operation::{CancellationOrigin, ControlCause};
use common::{
    GltfTestOptions, TestResult, assert_gltf_failure, assert_resource_limit, controlled_import,
    expected_error, import_assets, import_assets_result, import_assets_result_with_control,
};
use std::sync::atomic::Ordering;

const GRAPH_POLL_INTERVAL: usize = 4_096;
const CANONICAL_IMPORT_STACK_BUDGET: usize = 2 * 1024 * 1024;

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
    options: GltfTestOptions,
) -> TestResult<ImportResult<ImportReport>> {
    let (result, diagnostics) =
        import_assets_result("graph.gltf", [("graph.gltf", document)], options)?;
    assert!(diagnostics.is_empty());
    Ok(result)
}

fn controlled_graph_result(
    document: &[u8],
    options: GltfTestOptions,
    cancel_on: Option<usize>,
) -> TestResult<(ImportResult<ImportReport>, usize)> {
    let (control, polls) = controlled_import(cancel_on);
    let (result, diagnostics) = import_assets_result_with_control(
        "graph.gltf",
        [("graph.gltf", document.to_vec())],
        options,
        control,
    )?;
    assert!(diagnostics.is_empty());
    Ok((result, polls.load(Ordering::SeqCst)))
}

fn assert_cancelled(failure: &ImportFailure) {
    assert!(
        matches!(
            failure.cause(),
            ImportCause::Scope(abort)
                if matches!(
                    abort.cause(),
                    ScopeAbortCause::Control(ControlCause::Cancelled {
                        origin: CancellationOrigin::Signal,
                    })
                )
        ),
        "expected signal cancellation, got {:?}",
        failure.cause()
    );
}

fn cancelled_graph_failure(document: &[u8], cancel_on: usize) -> TestResult<ImportFailure> {
    let (result, polls) =
        controlled_graph_result(document, GltfTestOptions::memory(), Some(cancel_on))?;
    let failure = expected_error(result)?;
    assert_cancelled(&failure);
    assert_eq!(polls, cancel_on);
    Ok(failure)
}

fn cancelled_dimension_counts(
    document: &[u8],
    cancel_on: usize,
    dimension: ImportDimension,
) -> TestResult<(u64, u64)> {
    let failure = cancelled_graph_failure(document, cancel_on)?;
    Ok(dimension_counts(&failure, dimension))
}

fn first_cancelled_prefix_at_least(
    document: &[u8],
    total_polls: usize,
    dimension: ImportDimension,
    target_consumed: u64,
) -> TestResult<(usize, (u64, u64))> {
    let mut low = 1usize;
    let mut high = total_polls;
    while low < high {
        let middle = low + (high - low) / 2;
        let (_, consumed) = cancelled_dimension_counts(document, middle, dimension)?;
        if consumed >= target_consumed {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    Ok((low, cancelled_dimension_counts(document, low, dimension)?))
}

fn first_cancelled_attempt_at_least(
    document: &[u8],
    total_polls: usize,
    dimension: ImportDimension,
    target_attempted: u64,
) -> TestResult<(usize, ImportFailure)> {
    let mut low = 1usize;
    let mut high = total_polls;
    while low < high {
        let middle = low + (high - low) / 2;
        let failure = cancelled_graph_failure(document, middle)?;
        if dimension_counts(&failure, dimension).0 >= target_attempted {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    let failure = cancelled_graph_failure(document, low)?;
    assert!(dimension_counts(&failure, dimension).0 >= target_attempted);
    Ok((low, failure))
}

fn assert_parse_failure(document: Vec<u8>, expected_message: &str) -> TestResult<()> {
    let error = expected_error(import_graph_result(document, GltfTestOptions::memory())?)?;
    assert_gltf_failure(
        &error,
        GltfFailureCode::InvalidDocument,
        Some(expected_message),
    )
}

fn assert_limit_failure(
    document: Vec<u8>,
    options: GltfTestOptions,
    expected_limit: &'static str,
) -> TestResult<()> {
    let error = expected_error(import_graph_result(document, options)?)?;
    assert_resource_limit(&error, expected_limit);
    Ok(())
}

fn dimension_counts(error: &ImportFailure, dimension: ImportDimension) -> (u64, u64) {
    let snapshot = error
        .ledger()
        .dimension(dimension)
        .expect("tested graph dimension is configured");
    (snapshot.attempted(), snapshot.consumed())
}

#[test]
fn large_node_and_scene_root_cancellation_preserves_exact_completed_prefixes() -> TestResult<()> {
    let item_count = GRAPH_POLL_INTERVAL + 1;
    let roots = (0..item_count)
        .map(|index| index.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let nodes = (0..item_count).map(|_| "{}").collect::<Vec<_>>().join(", ");
    let document = scene_with_roots(&roots, &nodes);
    let (calibration, total_polls) =
        controlled_graph_result(&document, GltfTestOptions::memory(), None)?;
    let _report = calibration?;
    assert!(total_polls > 0);

    let (_, node_counts) = first_cancelled_prefix_at_least(
        &document,
        total_polls,
        ImportDimension::GraphNodes,
        GRAPH_POLL_INTERVAL as u64,
    )?;
    assert_eq!(node_counts, (item_count as u64, GRAPH_POLL_INTERVAL as u64));

    let root_prefix = (item_count + 1 + GRAPH_POLL_INTERVAL) as u64;
    let (_, root_counts) = first_cancelled_prefix_at_least(
        &document,
        total_polls,
        ImportDimension::GraphWork,
        root_prefix,
    )?;
    assert_eq!(root_counts, ((item_count * 2 + 1) as u64, root_prefix));
    Ok(())
}

#[test]
fn scene_graph_work_is_admitted_before_scene_root_list_storage() -> TestResult<()> {
    let scene_count = GRAPH_POLL_INTERVAL;
    let scenes = (0..scene_count)
        .map(|_| r#"{ "nodes": [] }"#)
        .collect::<Vec<_>>()
        .join(", ");
    let document = graph_document(&format!(r#""scene": 0, "scenes": [{scenes}],"#), "");

    let mut calibration_options = GltfTestOptions::memory();
    calibration_options.limits.max_graph_work = 0;
    let (calibration, total_polls) = controlled_graph_result(&document, calibration_options, None)?;
    let calibration_failure = expected_error(calibration)?;
    assert_resource_limit(&calibration_failure, "max_graph_work");
    assert!(total_polls > 1);

    let (admission_poll, admission_failure) =
        first_cancelled_attempt_at_least(&document, total_polls, ImportDimension::GraphWork, 1)?;
    assert_eq!(
        dimension_counts(&admission_failure, ImportDimension::GraphWork),
        (1, 0)
    );
    assert!(admission_poll > 1);
    let pre_admission_failure = cancelled_graph_failure(&document, admission_poll - 1)?;
    assert_eq!(
        dimension_counts(&pre_admission_failure, ImportDimension::GraphWork),
        (0, 0)
    );
    let pre_scene_root_peak = pre_admission_failure.ledger().peak_live_bytes();

    let mut options = GltfTestOptions::memory();
    options.limits.max_graph_work = 0;
    options.limits.max_live_bytes = pre_scene_root_peak;
    let failure = expected_error(import_graph_result(document, options)?)?;
    assert_resource_limit(&failure, "max_graph_work");
    assert_eq!(
        dimension_counts(&failure, ImportDimension::GraphWork),
        (1, 0)
    );
    assert_eq!(failure.ledger().peak_live_bytes(), pre_scene_root_peak);
    Ok(())
}

#[test]
fn imports_valid_forest_once_without_a_declared_scene() -> TestResult<()> {
    let document = graph_document(
        "",
        r#"{ "name": "RootA", "children": [1] },
           { "name": "ChildA" },
           { "name": "RootB" }"#,
    );
    let (report, diagnostics) = import_assets(
        "graph.gltf",
        [("graph.gltf", document)],
        GltfTestOptions::memory(),
    )?;

    assert!(diagnostics.is_empty());
    assert!(report.scene().canonical_block_count() > 0);
    assert_eq!(
        report
            .source_scope_record()
            .expect("source-scope evidence")
            .selection_outcome(),
        &SourceScopeSelectionOutcome::parentless_node_projection(
            AssetPath::new("graph.gltf").unwrap()
        )
    );
    Ok(())
}

#[test]
fn selects_the_only_scene_when_no_default_scene_is_declared() -> TestResult<()> {
    let document = graph_document(
        r#""scenes": [{ "nodes": [1] }],"#,
        r#"{ "name": "Unselected" }, { "name": "Selected" }"#,
    );
    let (report, diagnostics) = import_assets(
        "graph.gltf",
        [("graph.gltf", document)],
        GltfTestOptions::memory(),
    )?;

    assert!(diagnostics.is_empty());
    assert!(report.scene().canonical_block_count() > 0);
    assert_eq!(
        report
            .source_scope_record()
            .expect("source-scope evidence")
            .selection_outcome(),
        &SourceScopeSelectionOutcome::declared_scene(AssetPath::new("graph.gltf").unwrap(), 0,)
    );
    Ok(())
}

#[test]
fn allows_the_same_root_in_multiple_scenes() -> TestResult<()> {
    let document = graph_document(
        r#""scene": 1, "scenes": [{ "nodes": [0] }, { "nodes": [0] }],"#,
        r#"{ "name": "SharedRoot" }"#,
    );
    let (report, diagnostics) = import_assets(
        "graph.gltf",
        [("graph.gltf", document)],
        GltfTestOptions::memory(),
    )?;

    assert!(diagnostics.is_empty());
    assert!(report.scene().canonical_block_count() > 0);
    Ok(())
}

#[test]
fn preserves_selected_scene_root_and_child_order() -> TestResult<()> {
    let document = graph_document(
        r#""scene": 0, "scenes": [{ "nodes": [2, 0] }],"#,
        r#"{ "name": "RootA", "children": [1] },
           { "name": "ChildA" },
           { "name": "RootB" }"#,
    );
    let (report, diagnostics) = import_assets(
        "graph.gltf",
        [("graph.gltf", document)],
        GltfTestOptions::memory(),
    )?;

    assert!(diagnostics.is_empty());
    assert!(report.scene().canonical_block_count() > 0);
    Ok(())
}

#[test]
fn rejects_a_self_cycle() -> TestResult<()> {
    assert_parse_failure(
        scene_with_roots("0", r#"{ "name": "Loop", "children": [0] }"#),
        "cycle",
    )
}

#[test]
fn rejects_a_mutual_cycle() -> TestResult<()> {
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
fn rejects_a_shared_child() -> TestResult<()> {
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
fn shared_child_failure_preserves_the_validated_graph_prefix() -> TestResult<()> {
    let document = scene_with_roots(
        "0, 1",
        r#"{ "name": "A", "children": [2] },
           { "name": "B", "children": [2] },
           { "name": "Shared" }"#,
    );
    let error = expected_error(import_graph_result(document, GltfTestOptions::memory())?)?;

    assert_gltf_failure(
        &error,
        GltfFailureCode::InvalidDocument,
        Some("multiple parents"),
    )?;
    assert_eq!(
        dimension_counts(&error, ImportDimension::GraphNodes),
        (3, 3)
    );
    assert_eq!(
        dimension_counts(&error, ImportDimension::GraphEdges),
        (2, 1)
    );
    assert_eq!(dimension_counts(&error, ImportDimension::GraphWork), (5, 4));
    assert_eq!(
        dimension_counts(&error, ImportDimension::GraphDepth),
        (0, 0)
    );
    Ok(())
}

#[test]
fn graph_fuzz_corpus_terminates_with_structured_failures() -> TestResult<()> {
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
fn rejects_many_roots_sharing_one_child() -> TestResult<()> {
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
fn rejects_a_layered_diamond() -> TestResult<()> {
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
fn rejects_an_invalid_graph_before_opening_required_buffers() -> TestResult<()> {
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
fn rejects_a_duplicate_scene_root() -> TestResult<()> {
    assert_parse_failure(
        scene_with_roots("0, 0", r#"{ "name": "Duplicated" }"#),
        "duplicate root",
    )
}

#[test]
fn rejects_a_scene_root_that_is_also_a_child() -> TestResult<()> {
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
fn builds_a_wide_fanout_without_duplicate_nodes() -> TestResult<()> {
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
    let (report, diagnostics) = import_assets(
        "graph.gltf",
        [("graph.gltf", document)],
        GltfTestOptions::memory(),
    )?;

    assert!(diagnostics.is_empty());
    assert!(report.scene().canonical_block_count() > 0);
    let graph_nodes = report
        .ledger()
        .dimension(ImportDimension::GraphNodes)
        .expect("graph-node accounting is configured");
    assert_eq!(graph_nodes.consumed(), (child_count + 2) as u64);
    Ok(())
}

#[test]
fn accepts_exact_graph_limits_within_canonical_import_stack_budget() -> TestResult<()> {
    std::thread::Builder::new()
        .name("gltf-canonical-import-stack-budget".into())
        .stack_size(CANONICAL_IMPORT_STACK_BUDGET)
        .spawn(|| -> TestResult<()> {
            let document = scene_with_roots(
                "0",
                r#"{ "name": "Root", "children": [1] },
                   { "name": "Middle", "children": [2] },
                   { "name": "Leaf" }"#,
            );
            let mut options = GltfTestOptions::memory();
            options.limits.max_graph_nodes = 4;
            options.limits.max_graph_edges = 3;
            options.limits.max_graph_depth = 4;
            options.limits.max_graph_work = 7;

            let report = import_graph_result(document, options)??;
            assert!(report.scene().canonical_block_count() > 0);
            Ok(())
        })?
        .join()
        .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
}

#[test]
fn rejects_graph_node_limit_excess() -> TestResult<()> {
    let document = scene_with_roots("0", r#"{}"#);
    let mut options = GltfTestOptions::memory();
    options.limits.max_graph_nodes = 1;
    assert_limit_failure(document, options, "max_graph_nodes")
}

#[test]
fn rejects_graph_edge_limit_excess() -> TestResult<()> {
    let document = scene_with_roots("0", r#"{}"#);
    let mut options = GltfTestOptions::memory();
    options.limits.max_graph_edges = 0;
    assert_limit_failure(document, options, "max_graph_edges")
}

#[test]
fn rejects_graph_depth_limit_excess() -> TestResult<()> {
    let document = scene_with_roots("0", r#"{}"#);
    let mut options = GltfTestOptions::memory();
    options.limits.max_graph_depth = 1;
    assert_limit_failure(document, options, "max_graph_depth")
}

#[test]
fn rejects_graph_work_limit_excess() -> TestResult<()> {
    let document = graph_document("", r#"{}"#);
    let mut options = GltfTestOptions::memory();
    options.limits.max_graph_work = 2;
    assert_limit_failure(document, options, "max_graph_work")
}

#[test]
fn rejects_synthetic_edges_for_many_isolated_roots_before_scene_construction() -> TestResult<()> {
    let node_count = 64usize;
    let roots = (0..node_count)
        .map(|index| index.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let nodes = (0..node_count).map(|_| "{}").collect::<Vec<_>>().join(", ");
    let document = scene_with_roots(&roots, &nodes);
    let mut options = GltfTestOptions::memory();
    options.limits.max_graph_edges = 0;

    assert_limit_failure(document, options, "max_graph_edges")
}
