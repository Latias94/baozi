use super::*;
use crate::provenance::{Provenance, test_provenance_fixture};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

struct ReservationDropProbe(Arc<AtomicU8>);

impl Drop for ReservationDropProbe {
    fn drop(&mut self) {
        assert_eq!(
            self.0.swap(2, Ordering::SeqCst),
            1,
            "reservation companion must drop after staged node storage"
        );
    }
}

struct TransferredReservationDropProbe(Arc<AtomicBool>);

impl Drop for TransferredReservationDropProbe {
    fn drop(&mut self) {
        assert!(
            !self.0.swap(true, Ordering::SeqCst),
            "transferred reservation must be released exactly once"
        );
    }
}

struct PublicationCompanion {
    reservation: Option<TransferredReservationDropProbe>,
}

fn scene_builder() -> (CanonicalSceneBuilder, Provenance) {
    let (provenance_arena, provenance) = test_provenance_fixture();
    let builder = CanonicalSceneBuilder::try_new(
        SceneSpace::unknown(),
        ExtensionSchemaSnapshot::empty(),
        provenance_arena,
        provenance,
        DerivedAbsenceReason::NotProvided,
    )
    .expect("scene builder fixture should allocate");
    (builder, provenance)
}

fn node_descriptor<'batch, C, const N: usize>(
    batch: &CanonicalNodeBatch<'batch, C>,
    provenance: Provenance,
    name: &'static str,
    children: [CanonicalNodeSlot<'batch>; N],
) -> CanonicalNodeDescriptor<'batch> {
    let plan = batch
        .node_children_plan(N)
        .expect("node children should have a capacity plan");
    let children = batch
        .prepare_node_children(plan, children)
        .expect("node children should prepare");
    CanonicalNodeDescriptor {
        name: Some(EntityName::try_new(name).expect("fixture name should be valid")),
        authored_transform: AuthoredTransform::identity_trs(),
        children,
        camera: None,
        light: None,
        provenance: provenance.into(),
        extensions: Vec::new(),
    }
}

fn standalone_node(provenance: Provenance) -> Node {
    Node::try_new(
        None,
        AuthoredTransform::identity_trs(),
        [],
        None,
        None,
        provenance,
        [],
    )
    .expect("standalone node fixture should allocate")
}

struct LyingNodeSlots<'batch, const N: usize> {
    inner: std::array::IntoIter<CanonicalNodeSlot<'batch>, N>,
    reported: usize,
}

impl<'batch, const N: usize> Iterator for LyingNodeSlots<'batch, N> {
    type Item = CanonicalNodeSlot<'batch>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.reported, Some(self.reported))
    }
}

impl<const N: usize> ExactSizeIterator for LyingNodeSlots<'_, N> {}

fn prepare_two_node_tree<'owner>(
    builder: &'owner mut CanonicalSceneBuilder,
    provenance: Provenance,
) -> PreparedCanonicalNodeBatch<'owner> {
    let plan = builder
        .node_batch_plan(2)
        .expect("two nodes should have a batch plan");
    builder
        .prepare_node_batch(plan, |batch| {
            let root = batch.slot(0)?;
            let child = batch.slot(1)?;
            batch.initialize_node(root, node_descriptor(batch, provenance, "root", [child]))?;
            batch.initialize_node(child, node_descriptor(batch, provenance, "child", []))?;
            batch.set_root(root)
        })
        .expect("complete node batch should prepare")
}

fn prepare_drop_order_batch<'owner>(
    builder: &'owner mut CanonicalSceneBuilder,
    provenance: Provenance,
    state: Arc<AtomicU8>,
) -> PreparedCanonicalNodeBatch<'owner, ReservationDropProbe> {
    let plan = builder.node_batch_plan(1).unwrap();
    builder
        .prepare_node_batch_coupled(plan, ReservationDropProbe(Arc::clone(&state)), |batch| {
            batch.set_storage_drop_probe_for_test(state);
            let root = batch.slot(0)?;
            batch.initialize_node(root, node_descriptor(batch, provenance, "root", []))?;
            batch.set_root(root)
        })
        .unwrap()
}

#[test]
fn node_batch_preserves_dense_order_and_forward_children() {
    let (mut builder, provenance) = scene_builder();
    let prepared = prepare_two_node_tree(&mut builder, provenance);
    let committed = prepared.commit();

    assert_eq!(committed.root().index(), 0);
    assert_eq!(committed.node_id(0).unwrap().index(), 0);
    assert_eq!(committed.node_id(1).unwrap().index(), 1);
    assert_eq!(builder.data.nodes.len(), 2);
    assert_eq!(
        builder.data.nodes[0]
            .name()
            .map(crate::entity::EntityName::as_str),
        Some("root")
    );
    assert_eq!(
        builder.data.nodes[1]
            .name()
            .map(crate::entity::EntityName::as_str),
        Some("child")
    );
    assert_eq!(
        builder.data.nodes[0].children().collect::<Vec<_>>(),
        [NodeIndex::from_u32(1)]
    );
}

#[test]
fn node_batch_rejects_foreign_skipped_and_duplicate_slots_before_publication() {
    let (mut builder, provenance) = scene_builder();
    let plan = builder.node_batch_plan(2).unwrap();
    let error = builder
        .prepare_node_batch(plan, |batch| {
            let foreign = batch.foreign_slot_for_test(0);
            batch.initialize_node(foreign, node_descriptor(batch, provenance, "foreign", []))
        })
        .unwrap_err();
    assert_eq!(error, CanonicalSceneBuildError::ForeignNodeBatchSlot);
    assert!(builder.data.nodes.is_empty());
    assert_eq!(builder.data.root, None);

    let plan = builder.node_batch_plan(2).unwrap();
    let error = builder
        .prepare_node_batch(plan, |batch| {
            let skipped = batch.slot(1)?;
            batch.initialize_node(skipped, node_descriptor(batch, provenance, "skipped", []))
        })
        .unwrap_err();
    assert_eq!(
        error,
        CanonicalSceneBuildError::NodeBatchInitializationOrder {
            expected: 0,
            actual: 1,
        }
    );
    assert!(builder.data.nodes.is_empty());

    let plan = builder.node_batch_plan(2).unwrap();
    let error = builder
        .prepare_node_batch(plan, |batch| {
            let first = batch.slot(0)?;
            batch.initialize_node(first, node_descriptor(batch, provenance, "first", []))?;
            batch.initialize_node(first, node_descriptor(batch, provenance, "duplicate", []))
        })
        .unwrap_err();
    assert_eq!(
        error,
        CanonicalSceneBuildError::NodeBatchInitializationOrder {
            expected: 1,
            actual: 0,
        }
    );
    assert!(builder.data.nodes.is_empty());
}

#[test]
fn incomplete_node_batch_and_missing_root_leave_the_builder_pristine() {
    let (mut builder, provenance) = scene_builder();
    let plan = builder.node_batch_plan(2).unwrap();
    let error = builder
        .prepare_node_batch(plan, |batch| {
            let root = batch.slot(0)?;
            batch.initialize_node(root, node_descriptor(batch, provenance, "root", []))?;
            batch.set_root(root)
        })
        .unwrap_err();
    assert_eq!(
        error,
        CanonicalSceneBuildError::IncompleteNodeBatch {
            expected: 2,
            actual: 1,
        }
    );
    assert!(builder.data.nodes.is_empty());
    assert_eq!(builder.data.root, None);

    let plan = builder.node_batch_plan(1).unwrap();
    let error = builder
        .prepare_node_batch(plan, |batch| {
            let root = batch.slot(0)?;
            batch.initialize_node(root, node_descriptor(batch, provenance, "root", []))
        })
        .unwrap_err();
    assert_eq!(error, CanonicalSceneBuildError::MissingNodeBatchRoot);
    assert!(builder.data.nodes.is_empty());
    assert_eq!(builder.data.root, None);
}

#[test]
fn callback_error_and_panic_discard_the_entire_node_batch() {
    let (mut builder, provenance) = scene_builder();
    let plan = builder.node_batch_plan(1).unwrap();
    let expected = CanonicalSceneBuildError::MissingNodeBatchRoot;
    let error = builder
        .prepare_node_batch(plan, |batch| {
            let root = batch.slot(0)?;
            batch.initialize_node(root, node_descriptor(batch, provenance, "root", []))?;
            Err(expected)
        })
        .unwrap_err();
    assert_eq!(error, expected);
    assert!(builder.data.nodes.is_empty());
    assert_eq!(builder.data.root, None);

    let plan = builder.node_batch_plan(1).unwrap();
    let panic = catch_unwind(AssertUnwindSafe(|| {
        let _ = builder.prepare_node_batch(plan, |batch| {
            let root = batch.slot(0)?;
            batch.initialize_node(root, node_descriptor(batch, provenance, "root", []))?;
            panic!("injected node producer panic");
        });
    }));
    assert!(panic.is_err());
    assert!(builder.data.nodes.is_empty());
    assert_eq!(builder.data.root, None);
}

#[test]
fn prepared_node_batch_separates_footprints_and_adopts_the_staging_vec() {
    let (mut builder, provenance) = scene_builder();
    let plan = builder.node_batch_plan(2).unwrap();
    assert!(plan.collection_envelope_bytes().unwrap() > 0);
    assert!(plan.value_storage_envelope_bytes() > 0);

    let prepared = prepare_two_node_tree(&mut builder, provenance);
    assert!(prepared.collection_retained_allocation_bytes() > 0);
    assert!(
        prepared.collection_retained_allocation_bytes()
            <= plan.collection_envelope_bytes().unwrap()
    );
    assert!(prepared.value_storage_retained_allocation_bytes() > 0);
    assert!(
        prepared.value_storage_retained_allocation_bytes() <= plan.value_storage_envelope_bytes()
    );
    assert!(prepared.nested_payload_retained_allocation_bytes() > 0);
    let staging_ptr = prepared.nodes_ptr_for_test();
    let staging_capacity = prepared.staging.nodes.capacity();

    prepared.commit();

    assert_eq!(builder.data.nodes.as_ptr(), staging_ptr);
    assert_eq!(builder.data.nodes.len(), 2);
    assert_eq!(builder.data.nodes.capacity(), staging_capacity);
}

#[test]
fn node_slots_and_root_enforce_batch_ownership_and_bounds() {
    let (mut builder, provenance) = scene_builder();
    let plan = builder.node_batch_plan(1).unwrap();
    let error = builder
        .prepare_node_batch(plan, |batch| {
            let foreign = batch.foreign_slot_for_test(0);
            batch.set_root(foreign)
        })
        .unwrap_err();
    assert_eq!(error, CanonicalSceneBuildError::ForeignNodeBatchSlot);
    assert!(builder.data.nodes.is_empty());

    let plan = builder.node_batch_plan(1).unwrap();
    let error = builder
        .prepare_node_batch(plan, |batch| batch.slot(1).map(|_| ()))
        .unwrap_err();
    assert_eq!(
        error,
        CanonicalSceneBuildError::NodeBatchSlotOutOfRange {
            actual: 1,
            node_count: 1,
        }
    );

    let plan = builder.node_batch_plan(1).unwrap();
    let error = builder
        .prepare_node_batch(plan, |batch| {
            let root = batch.slot(0)?;
            batch.initialize_node(root, node_descriptor(batch, provenance, "root", []))?;
            batch.set_root(root)?;
            batch.set_root(root)
        })
        .unwrap_err();
    assert_eq!(error, CanonicalSceneBuildError::DuplicateNodeBatchRoot);
    assert!(builder.data.nodes.is_empty());
}

#[test]
fn node_children_reject_foreign_slots_and_plan_count_mismatches() {
    let (mut builder, _) = scene_builder();
    let plan = builder.node_batch_plan(1).unwrap();
    let error = builder
        .prepare_node_batch(plan, |batch| {
            let children_plan = batch.node_children_plan(1)?;
            batch
                .prepare_node_children(children_plan, [batch.foreign_slot_for_test(0)])
                .map(|_| ())
        })
        .unwrap_err();
    assert_eq!(error, CanonicalSceneBuildError::ForeignNodeBatchSlot);
    assert!(builder.data.nodes.is_empty());

    let plan = builder.node_batch_plan(1).unwrap();
    let error = builder
        .prepare_node_batch(plan, |batch| {
            let children_plan = batch.node_children_plan(1)?;
            batch.prepare_node_children(children_plan, []).map(|_| ())
        })
        .unwrap_err();
    assert_eq!(
        error,
        CanonicalSceneBuildError::NodeBatchChildrenCountMismatch {
            expected: 1,
            actual: 0,
        }
    );
    assert!(builder.data.nodes.is_empty());
}

#[test]
fn node_children_recheck_a_misbehaving_exact_size_iterator_without_growth() {
    let (mut builder, _) = scene_builder();
    let plan = builder.node_batch_plan(1).unwrap();
    let error = builder
        .prepare_node_batch(plan, |batch| {
            let child = batch.slot(0)?;
            let children_plan = batch.node_children_plan(1)?;
            let children = LyingNodeSlots {
                inner: [child, child].into_iter(),
                reported: 1,
            };
            batch
                .prepare_node_children(children_plan, children)
                .map(|_| ())
        })
        .unwrap_err();
    assert_eq!(
        error,
        CanonicalSceneBuildError::NodeBatchChildrenCountMismatch {
            expected: 1,
            actual: 2,
        }
    );
    assert!(builder.data.nodes.is_empty());
    assert_eq!(builder.data.nodes.capacity(), 0);

    let plan = builder.node_batch_plan(1).unwrap();
    let error = builder
        .prepare_node_batch(plan, |batch| {
            let children_plan = batch.node_children_plan(1)?;
            let children = LyingNodeSlots {
                inner: [].into_iter(),
                reported: 1,
            };
            batch
                .prepare_node_children(children_plan, children)
                .map(|_| ())
        })
        .unwrap_err();
    assert_eq!(
        error,
        CanonicalSceneBuildError::NodeBatchChildrenCountMismatch {
            expected: 1,
            actual: 0,
        }
    );
    assert!(builder.data.nodes.is_empty());
    assert_eq!(builder.data.nodes.capacity(), 0);
}

#[test]
fn failed_or_dropped_preparation_leaves_the_builder_retryable() {
    let (mut builder, provenance) = scene_builder();
    let mut underestimated = builder.node_batch_plan(1).unwrap();
    underestimated.value_storage_envelope_bytes = underestimated
        .value_storage_envelope_bytes
        .checked_sub(1)
        .expect("node shared storage footprint is nonzero");
    let error = builder
        .prepare_node_batch(underestimated, |batch| {
            let root = batch.slot(0)?;
            batch.initialize_node(root, node_descriptor(batch, provenance, "root", []))?;
            batch.set_root(root)
        })
        .unwrap_err();
    assert!(matches!(
        error,
        CanonicalSceneBuildError::PreparedAllocationUnderestimated {
            collection: CanonicalSceneCollection::Nodes,
            allocation: CanonicalPreparedAllocation::ValueStorage,
            ..
        }
    ));
    assert!(builder.data.nodes.is_empty());
    assert_eq!(builder.data.nodes.capacity(), 0);

    let plan = builder.node_batch_plan(1).unwrap();
    crate::shared_storage::fail_next_allocation_for_test();
    let error = builder
        .prepare_node_batch(plan, |batch| {
            let root = batch.slot(0)?;
            batch.initialize_node(root, node_descriptor(batch, provenance, "root", []))?;
            batch.set_root(root)
        })
        .unwrap_err();
    assert_eq!(
        error,
        CanonicalSceneBuildError::Node(NodeError::PayloadAllocationFailed)
    );
    assert!(builder.data.nodes.is_empty());
    assert_eq!(builder.data.nodes.capacity(), 0);

    let prepared = prepare_two_node_tree(&mut builder, provenance);
    drop(prepared);
    assert!(builder.data.nodes.is_empty());
    assert_eq!(builder.data.nodes.capacity(), 0);
    assert_eq!(builder.data.root, None);

    prepare_two_node_tree(&mut builder, provenance).commit();
    assert_eq!(builder.data.nodes.len(), 2);
}

#[test]
fn foreign_plan_and_non_pristine_builder_are_rejected_without_allocation() {
    let (mut first, provenance) = scene_builder();
    let (second, _) = scene_builder();
    let foreign = second.node_batch_plan(1).unwrap();
    assert_eq!(
        first.prepare_node_batch(foreign, |_| Ok(())).unwrap_err(),
        CanonicalSceneBuildError::ForeignNodeBatchPlan
    );
    assert!(first.data.nodes.is_empty());

    first.try_push_node(standalone_node(provenance)).unwrap();
    assert!(matches!(
        first.node_batch_plan(1),
        Err(CanonicalSceneBuildError::NodeBatchNotPristine { .. })
    ));
}

#[test]
fn node_batch_plan_boundaries_and_stale_pristine_state_are_typed() {
    let (mut builder, provenance) = scene_builder();
    assert_eq!(
        builder.node_batch_plan(0).unwrap_err(),
        CanonicalSceneBuildError::EmptyNodeBatch
    );

    let plan = builder.node_batch_plan(1).unwrap();
    let error = builder
        .prepare_node_batch(plan, |batch| {
            batch.node_children_plan(crate::entity::MAX_NODE_CHILDREN + 1)?;
            Ok(())
        })
        .unwrap_err();
    assert_eq!(
        error,
        CanonicalSceneBuildError::Node(NodeError::LimitExceeded {
            resource: crate::entity::EntityAllocation::NodeChildren,
            maximum: crate::entity::MAX_NODE_CHILDREN,
        })
    );
    assert!(builder.data.nodes.is_empty());
    assert_eq!(builder.data.nodes.capacity(), 0);

    let stale = builder.node_batch_plan(1).unwrap();
    builder.try_push_node(standalone_node(provenance)).unwrap();
    assert!(matches!(
        builder.prepare_node_batch(stale, |_| Ok(())),
        Err(CanonicalSceneBuildError::NodeBatchNotPristine { .. })
    ));
    assert_eq!(builder.data.nodes.len(), 1);
}

#[test]
fn callback_error_drops_staging_before_its_reservation_companion() {
    let (mut builder, provenance) = scene_builder();
    let state = Arc::new(AtomicU8::new(0));
    let plan = builder.node_batch_plan(1).unwrap();
    let expected = CanonicalSceneBuildError::MissingNodeBatchRoot;
    let error = builder
        .prepare_node_batch_coupled(plan, ReservationDropProbe(Arc::clone(&state)), |batch| {
            batch.set_storage_drop_probe_for_test(Arc::clone(&state));
            let root = batch.slot(0)?;
            batch.initialize_node(root, node_descriptor(batch, provenance, "root", []))?;
            Err(expected)
        })
        .unwrap_err();

    assert_eq!(error, expected);
    assert_eq!(state.load(Ordering::SeqCst), 2);
    assert!(builder.data.nodes.is_empty());
    assert_eq!(builder.data.nodes.capacity(), 0);
}

#[test]
fn companion_publication_failure_panic_and_drop_preserve_storage_order() {
    let (mut builder, provenance) = scene_builder();
    let state = Arc::new(AtomicU8::new(0));
    let prepared = prepare_drop_order_batch(&mut builder, provenance, Arc::clone(&state));
    let error = prepared
        .try_commit_coupled(|_companion, evidence| {
            assert!(evidence.collection_retained_allocation_bytes() > 0);
            assert!(evidence.value_storage_retained_allocation_bytes() > 0);
            assert!(evidence.nested_payload_retained_allocation_bytes() > 0);
            Err("publication rejected")
        })
        .unwrap_err();
    assert_eq!(error, "publication rejected");
    assert_eq!(state.load(Ordering::SeqCst), 2);
    assert!(builder.data.nodes.is_empty());

    let (mut builder, provenance) = scene_builder();
    let state = Arc::new(AtomicU8::new(0));
    let prepared = prepare_drop_order_batch(&mut builder, provenance, Arc::clone(&state));
    let panic = catch_unwind(AssertUnwindSafe(|| {
        let _ = prepared.try_commit_coupled(|_companion, _evidence| -> Result<(), Infallible> {
            panic!("injected companion publication panic")
        });
    }));
    assert!(panic.is_err());
    assert_eq!(state.load(Ordering::SeqCst), 2);
    assert!(builder.data.nodes.is_empty());

    let (mut builder, provenance) = scene_builder();
    let state = Arc::new(AtomicU8::new(0));
    drop(prepare_drop_order_batch(
        &mut builder,
        provenance,
        Arc::clone(&state),
    ));
    assert_eq!(state.load(Ordering::SeqCst), 2);
    assert!(builder.data.nodes.is_empty());
}

#[test]
fn companion_publication_transfers_coverage_before_node_adoption() {
    let (mut builder, provenance) = scene_builder();
    let reservation_dropped = Arc::new(AtomicBool::new(false));
    let companion = PublicationCompanion {
        reservation: Some(TransferredReservationDropProbe(Arc::clone(
            &reservation_dropped,
        ))),
    };
    let plan = builder.node_batch_plan(1).unwrap();
    let prepared = builder
        .prepare_node_batch_coupled(plan, companion, |batch| {
            let root = batch.slot(0)?;
            batch.initialize_node(root, node_descriptor(batch, provenance, "root", []))?;
            batch.set_root(root)
        })
        .unwrap();
    let expected_evidence = prepared.allocation_evidence();
    let mut published_reservation = None;

    let committed = prepared
        .try_commit_coupled(|companion, evidence| {
            assert_eq!(evidence, expected_evidence);
            published_reservation = companion.reservation.take();
            Ok::<(), Infallible>(())
        })
        .unwrap();

    assert_eq!(committed.root().index(), 0);
    assert_eq!(builder.data.nodes.len(), 1);
    assert!(!reservation_dropped.load(Ordering::SeqCst));
    drop(builder);
    assert!(!reservation_dropped.load(Ordering::SeqCst));
    drop(published_reservation);
    assert!(reservation_dropped.load(Ordering::SeqCst));
}
