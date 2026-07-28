use crate::SceneValidationLimits;
use crate::attribute::{AttributeDomain, AttributeSemantic};

use super::super::plan::geometry::validate_geometry_batch_with_control;
use super::super::scene::{
    SceneAuthoringAggregateError, SceneAuthoringAggregateLimit, SceneInputPreflightError,
    terminal_attribution_slots_for_shape, validate_scene_aggregate,
};
use super::super::{
    AuthoringRange, GeometryAttributeAuthoring, GeometryAuthoringBatch,
    GeometryLogicalMeshAuthoring, GeometryPrimitiveAuthoring, GeometryTopologyAuthoring,
    NodeLocalId, Presence,
};
use super::{TestControl, trs, validate};

#[test]
fn scene_attribution_shape_covers_dynamic_canonical_members() {
    assert_eq!(
        terminal_attribution_slots_for_shape(0, 0, 1, 12, 12, 0)
            .expect("the Scene shape is representable"),
        67,
    );
}

#[test]
fn scene_attribution_shape_rejects_checked_arithmetic_overflow() {
    assert!(matches!(
        terminal_attribution_slots_for_shape(0, 0, 0, usize::MAX, 0, 0),
        Err(SceneInputPreflightError::AccountingOverflow {
            dimension: "primitive payload block upper bound",
        })
    ));
    assert!(matches!(
        terminal_attribution_slots_for_shape(0, 0, 0, 0, usize::MAX, 0),
        Err(SceneInputPreflightError::AccountingOverflow {
            dimension: "attribute payload block upper bound",
        })
    ));
}

#[test]
fn scene_aggregate_accepts_exact_limits_and_rejects_each_one_short() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let positions = [0.0_f32, 0.0, 0.0];
    let attributes = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &positions,
        Presence::Absent,
    )];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let geometry_input =
        GeometryAuthoringBatch::new(0, &logical_meshes, &primitives, &attributes, &[]);
    let mut control = TestControl::default();
    let node_batch = validate(
        Presence::Present(&roots),
        &nodes,
        SceneValidationLimits::default(),
        &mut control,
    )
    .expect("node domain validates independently");
    let geometry_batch = validate_geometry_batch_with_control(
        geometry_input,
        nodes.len(),
        SceneValidationLimits::default(),
        &mut control,
    )
    .expect("geometry domain validates independently");

    let entities = node_batch
        .canonical_node_count()
        .checked_add(geometry_batch.entity_count())
        .expect("test entity total");
    let edges = node_batch
        .reference_count()
        .checked_add(geometry_batch.edge_count())
        .expect("test edge total");
    let references = node_batch
        .reference_count()
        .checked_add(geometry_batch.reference_count())
        .expect("test reference total");
    let semantic_cells = node_batch
        .semantic_cell_count()
        .checked_add(geometry_batch.semantic_cell_count())
        .expect("test semantic-cell total");
    let work = node_batch
        .validation_work()
        .checked_add(geometry_batch.validation_work())
        .expect("test work total");
    let canonical_bytes = node_batch
        .canonical_non_geometry_bytes()
        .expect("test node canonical bytes")
        .checked_add(geometry_batch.canonical_bytes())
        .expect("test canonical-byte total");
    let scratch_bytes = node_batch
        .scratch_bytes()
        .checked_add(geometry_batch.scratch_bytes())
        .expect("test scratch-byte total");
    let exact = SceneValidationLimits::default()
        .with_max_entities(entities)
        .with_max_edges(edges)
        .with_max_references(references)
        .with_max_payload_cells(semantic_cells)
        .with_max_work(work)
        .with_max_canonical_bytes(canonical_bytes)
        .with_max_scratch_bytes(scratch_bytes);
    assert!(validate_scene_aggregate(&node_batch, &geometry_batch, exact).is_ok());

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
            validate_scene_aggregate(&node_batch, &geometry_batch, one_short),
            Err(SceneAuthoringAggregateError::LimitExceeded {
                limit: actual,
                ..
            }) if actual == limit
        ));
    }

    drop(geometry_batch);
    drop(node_batch);
    assert_eq!(control.live.get(), 0);
}
