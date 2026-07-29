use crate::SceneValidationLimits;

use super::super::plan::animation::{
    AnimationTargetCatalog, validate_animation_batch_with_control,
};
use super::super::plan::appearance::validate_appearance_batch_with_control;
use super::super::plan::deformation::validate_deformation_batch_with_control;
use super::super::plan::geometry::validate_geometry_batch_with_control;
use super::super::scene::{
    SceneAuthoringAggregateError, SceneAuthoringAggregateLimit, SceneInputPreflight,
    SceneInputPreflightError, animation_limits_after_deformation, appearance_limits_after_nodes,
    deformation_limits_after_geometry, geometry_limits_after_appearance, validate_scene_aggregate,
};
use super::super::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, DeformationAuthoringBatch,
    GeometryAuthoringBatch, NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, Presence,
    SceneAuthoringBatch,
};
use super::{TestControl, trs, validate};

fn empty_scene_input<'input>(
    roots: Presence<&'input [NodeLocalId]>,
    nodes: &'input [NodeAuthoringNode],
) -> SceneAuthoringBatch<'input> {
    SceneAuthoringBatch::new(
        NodeAuthoringBatch::new(roots, nodes),
        AppearanceAuthoringBatch::empty(),
        GeometryAuthoringBatch::empty(),
        DeformationAuthoringBatch::empty(),
        AnimationAuthoringBatch::empty(),
    )
}

fn checked_total(values: [u64; 5]) -> u64 {
    values
        .into_iter()
        .try_fold(0_u64, u64::checked_add)
        .expect("test metric total is representable")
}

#[test]
fn scene_preflight_accounts_for_the_complete_five_domain_shape() {
    let empty_roots = [];
    let empty_nodes = [];
    let empty = SceneInputPreflight::try_new(
        empty_scene_input(Presence::Present(&empty_roots), &empty_nodes),
        SceneValidationLimits::default(),
    )
    .expect("the empty five-domain Scene shape is representable");
    assert_eq!(empty.terminal_attribution_slots(), 7);

    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let populated_nodes = SceneInputPreflight::try_new(
        empty_scene_input(Presence::Present(&roots), &nodes),
        SceneValidationLimits::default(),
    )
    .expect("the populated node shape is representable");
    assert_eq!(populated_nodes.terminal_attribution_slots(), 9);
}

#[test]
fn scene_preflight_rejects_real_input_limit_overflow() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];

    assert!(matches!(
        SceneInputPreflight::try_new(
            empty_scene_input(Presence::Present(&roots), &nodes),
            SceneValidationLimits::default().with_max_nodes(1),
        ),
        Err(SceneInputPreflightError::LimitExceeded {
            limit: "nodes",
            actual: 2,
            maximum: 1,
        })
    ));
    assert!(matches!(
        SceneInputPreflight::try_new(
            empty_scene_input(Presence::Present(&roots), &nodes),
            SceneValidationLimits::default().with_max_entities(1),
        ),
        Err(SceneInputPreflightError::LimitExceeded {
            limit: "entities",
            actual: 2,
            maximum: 1,
        })
    ));
}

#[test]
fn scene_aggregate_accepts_exact_limits_and_rejects_each_one_short() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let mut control = TestControl::default();
    let defaults = SceneValidationLimits::default();

    let node_batch = validate(Presence::Present(&roots), &nodes, defaults, &mut control)
        .expect("node domain validates independently");
    let appearance_batch = validate_appearance_batch_with_control(
        AppearanceAuthoringBatch::empty(),
        defaults,
        &mut control,
    )
    .expect("empty appearance domain validates independently");
    let geometry_batch = validate_geometry_batch_with_control(
        GeometryAuthoringBatch::empty(),
        nodes.len(),
        appearance_batch.material_count(),
        defaults,
        &mut control,
    )
    .expect("empty geometry domain validates independently");
    let deformation_batch = validate_deformation_batch_with_control(
        DeformationAuthoringBatch::empty(),
        &node_batch,
        &geometry_batch,
        defaults,
        &mut control,
    )
    .expect("empty deformation domain validates independently");
    let animation_targets = AnimationTargetCatalog::new(&node_batch, &deformation_batch);
    let animation_batch = validate_animation_batch_with_control(
        AnimationAuthoringBatch::empty(),
        &animation_targets,
        defaults,
        &mut control,
    )
    .expect("empty animation domain validates independently");

    let entities = checked_total([
        node_batch.canonical_node_count(),
        appearance_batch.entity_count(),
        geometry_batch.entity_count(),
        deformation_batch.entity_count(),
        animation_batch.entity_count(),
    ]);
    let edges = checked_total([
        node_batch.reference_count(),
        appearance_batch.edge_count(),
        geometry_batch.edge_count(),
        deformation_batch.edge_count(),
        animation_batch.edge_count(),
    ]);
    let references = checked_total([
        node_batch.reference_count(),
        appearance_batch.reference_count(),
        geometry_batch.reference_count(),
        deformation_batch.reference_count(),
        animation_batch.reference_count(),
    ]);
    let semantic_cells = checked_total([
        node_batch.semantic_cell_count(),
        appearance_batch.semantic_cell_count(),
        geometry_batch.semantic_cell_count(),
        deformation_batch.semantic_cell_count(),
        animation_batch.semantic_cell_count(),
    ]);
    let work = checked_total([
        node_batch.validation_work(),
        appearance_batch.validation_work(),
        geometry_batch.validation_work(),
        deformation_batch.validation_work(),
        animation_batch.validation_work(),
    ]);
    let canonical_bytes = checked_total([
        node_batch
            .canonical_non_geometry_bytes()
            .expect("test node canonical bytes are representable"),
        appearance_batch.canonical_bytes(),
        geometry_batch.canonical_bytes(),
        deformation_batch.canonical_bytes(),
        animation_batch.canonical_bytes(),
    ]);
    let scratch_bytes = checked_total([
        node_batch.scratch_bytes(),
        appearance_batch.scratch_bytes(),
        geometry_batch.scratch_bytes(),
        deformation_batch.scratch_bytes(),
        animation_batch.scratch_bytes(),
    ]);
    let exact = defaults
        .with_max_entities(entities)
        .with_max_edges(edges)
        .with_max_references(references)
        .with_max_payload_cells(semantic_cells)
        .with_max_work(work)
        .with_max_canonical_bytes(canonical_bytes)
        .with_max_scratch_bytes(scratch_bytes);

    let appearance_limits = appearance_limits_after_nodes(&node_batch, exact)
        .expect("node metrics fit the exact Scene limits");
    let geometry_limits = geometry_limits_after_appearance(&appearance_batch, appearance_limits)
        .expect("appearance metrics fit the remaining Scene limits");
    let deformation_limits = deformation_limits_after_geometry(&geometry_batch, geometry_limits)
        .expect("geometry metrics fit the remaining Scene limits");
    let animation_limits =
        animation_limits_after_deformation(&deformation_batch, deformation_limits)
            .expect("deformation metrics fit the remaining Scene limits");
    assert_eq!(
        animation_limits.max_entities(),
        animation_batch.entity_count()
    );
    assert_eq!(animation_limits.max_edges(), animation_batch.edge_count());
    assert_eq!(
        animation_limits.max_references(),
        animation_batch.reference_count()
    );
    assert_eq!(
        animation_limits.max_payload_cells(),
        animation_batch.semantic_cell_count()
    );
    assert_eq!(
        animation_limits.max_work(),
        animation_batch.validation_work()
    );
    assert_eq!(
        animation_limits.max_canonical_bytes(),
        animation_batch.canonical_bytes()
    );
    assert_eq!(
        animation_limits.max_scratch_bytes(),
        animation_batch.scratch_bytes()
    );

    assert!(
        validate_scene_aggregate(
            &node_batch,
            &appearance_batch,
            &geometry_batch,
            &deformation_batch,
            &animation_batch,
            exact,
        )
        .is_ok()
    );

    for (limit, one_short) in [
        (
            SceneAuthoringAggregateLimit::Entities,
            exact.with_max_entities(entities - 1),
        ),
        (
            SceneAuthoringAggregateLimit::Edges,
            exact.with_max_edges(edges - 1),
        ),
        (
            SceneAuthoringAggregateLimit::References,
            exact.with_max_references(references - 1),
        ),
        (
            SceneAuthoringAggregateLimit::SemanticCells,
            exact.with_max_payload_cells(semantic_cells - 1),
        ),
        (
            SceneAuthoringAggregateLimit::Work,
            exact.with_max_work(work - 1),
        ),
        (
            SceneAuthoringAggregateLimit::CanonicalBytes,
            exact.with_max_canonical_bytes(canonical_bytes - 1),
        ),
        (
            SceneAuthoringAggregateLimit::ScratchBytes,
            exact.with_max_scratch_bytes(scratch_bytes - 1),
        ),
    ] {
        assert!(matches!(
            validate_scene_aggregate(
                &node_batch,
                &appearance_batch,
                &geometry_batch,
                &deformation_batch,
                &animation_batch,
                one_short,
            ),
            Err(SceneAuthoringAggregateError::LimitExceeded {
                limit: actual,
                ..
            }) if actual == limit
        ));
    }

    drop(animation_batch);
    drop(deformation_batch);
    drop(geometry_batch);
    drop(appearance_batch);
    drop(node_batch);
    assert_eq!(control.live.get(), 0);
}
