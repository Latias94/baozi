//! Manual attribution only: the kernel without operation accounting is not a Scene operation.

use std::{convert::Infallible, hint::black_box, time::Instant};

use super::{Input, Mode, Output, PositionSource, VertexIds, WorkEvent, Workspace, generate_into};
use crate::precision::Estimate3;

#[test]
#[ignore = "manual optimized kernel attribution; no operation-accounting or publication guarantee"]
fn repeated_triangles_separate_kernel_and_position_reads_from_operation_cost() {
    const FACES: usize = 16_384;
    let positions = [[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let indices: Vec<_> = (0..FACES).flat_map(|_| [0_u32, 1, 2]).collect();
    let groups = vec![1_u32; FACES];

    let mut bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\ns 1\n".to_vec();
    for _ in 0..FACES {
        bytes.extend_from_slice(b"f 1 2 3\n");
    }
    let (scene, evidence) = crate::test_fixtures::import_obj(&bytes).into_parts();
    let primitive = scene
        .meshes()
        .next()
        .expect("one original mesh")
        .primitives()
        .next()
        .expect("one original primitive");
    assert_eq!(primitive.indices().expect("source indices"), indices);
    let attribute = primitive.positions().expect("source positions");
    assert_eq!(attribute.len(), positions.len());

    sample("slice", &positions[..], &indices, &groups);
    sample("attribute", attribute, &indices, &groups);
    drop(evidence);
    drop(scene);
}

fn sample<P: PositionSource>(label: &str, positions: P, indices: &[u32], groups: &[u32]) {
    let mut output = vec![[f32::NAN; 3]; indices.len()];
    let mut order = vec![usize::MAX; indices.len()];
    let mut crosses = vec![
        Estimate3 {
            value: [f64::NAN; 3],
            abs_error: [f64::NAN; 3],
        };
        groups.len()
    ];
    let mut samples = Vec::with_capacity(20);
    let mut work_actions = 0_u64;
    let mut checkpoints = 0_u64;
    for iteration in 0..23 {
        work_actions = 0;
        checkpoints = 0;
        let start = Instant::now();
        generate_into(
            black_box(Input {
                positions,
                vertex_ids: VertexIds::Indexed(indices),
                smoothing_groups: Some(groups),
            }),
            Mode::Smooth,
            Output::F32(&mut output),
            Workspace {
                corner_order: &mut order,
                vertex_cursors: None,
                face_crosses: Some(&mut crosses),
            },
            |event| {
                match event {
                    WorkEvent::Begin(_) => work_actions += 1,
                    WorkEvent::Complete => {}
                    WorkEvent::Checkpoint => checkpoints += 1,
                }
                Ok::<(), Infallible>(())
            },
        )
        .expect("the unchanged kernel accepts every original triangle");
        let elapsed = start.elapsed().as_nanos();
        black_box(&output);
        assert!(output.iter().all(|normal| *normal == [0.0, 0.0, 1.0]));
        if iteration >= 3 {
            samples.push(elapsed);
        }
    }
    println!(
        "{{\"schema\":\"baozi-normal-kernel-diagnostic/v3\",\"position_source\":\"{label}\",\"faces\":{},\"work_actions\":{work_actions},\"checkpoints\":{checkpoints},\"work_action_contract\":\"bounded-normal-actions/v2\",\"warmups\":3,\"samples_ns\":{samples:?},\"scope\":\"kernel only, preallocated workspace, local counting observer, no operation accounting or publication\"}}",
        groups.len()
    );
}

#[test]
#[ignore = "manual optimized operation attribution; empty actions are not normal generation"]
#[allow(
    clippy::result_large_err,
    reason = "the diagnostic measures the real host with its inline accounting and control causes"
)]
fn observed_unit_protocol_with_empty_actions_preserves_exact_work_evidence() {
    use baozi_operation::__retained_store_integration::activation::FiniteCanonicalOperationPublication;
    use baozi_operation::__retained_store_integration::authority::PreparedFiniteStoreEvidenceOnlyOutcome;

    use crate::operation::runtime;

    // Fixed historical pre-P3 control-site workload, not the new semantic normal-action count.
    const ACTIONS: u64 = 982_943;
    let (scene, source_evidence) = crate::test_fixtures::canonical_triangle_import().into_parts();
    let mut samples = Vec::with_capacity(20);
    for iteration in 0..23 {
        let (publication, (keys, state, elapsed)) = runtime::with_test_operation(
            &scene,
            crate::PostProcessControl::default(),
            |operation, keys| {
                let (state, elapsed) = {
                    operation
                        .with_action_execution::<_, baozi_operation::StoreRuntimeError>(
                            keys.work,
                            |actions| {
                                let initial = actions
                                    .work_counters(keys.work)
                                    .expect("initial work state");
                                let start = Instant::now();
                                for _ in 0..ACTIONS {
                                    actions.begin_action(1)?;
                                    black_box(());
                                    actions.complete_action()?;
                                }
                                let elapsed = start.elapsed().as_nanos();
                                actions.checkpoint()?;
                                let state = actions
                                    .work_counters(keys.work)
                                    .expect("completed work state");
                                assert_eq!(state.0, initial.0 + ACTIONS);
                                assert_eq!(state.1, initial.1 + ACTIONS);
                                assert_eq!(state.2, 0);
                                Ok((state, elapsed))
                            },
                        )
                        .expect("every actual empty-action prefix resolves")
                };
                (keys, state, elapsed)
            },
        );
        let FiniteCanonicalOperationPublication::EvidenceOnly(prepared) = publication else {
            panic!("every observed unit resolved before its unchanged publication");
        };
        let PreparedFiniteStoreEvidenceOnlyOutcome::Success(prepared) = prepared else {
            panic!("the unchanged empty-action operation must publish success");
        };
        let handoff = prepared.handoff();
        let facts = &handoff.facts;
        assert_eq!(facts.header().current_live_bytes(), 0);
        assert_eq!(facts.header().active_reservations(), 0);
        let terminal = facts.dimension(keys.work).expect("final ordinary work");
        assert_eq!(terminal.attempted(), state.0);
        assert_eq!(terminal.consumed(), state.1);
        if iteration >= 3 {
            samples.push(elapsed);
        }
    }
    drop(source_evidence);
    drop(scene);
    println!(
        "{{\"schema\":\"baozi-observed-unit-diagnostic/v3\",\"actions\":{ACTIONS},\"action_count_basis\":\"historical-pre-P3-control-sites\",\"warmups\":3,\"samples_ns\":{samples:?},\"scope\":\"actual scene edit action host with empty actions; binding, entry, final checkpoint and publication outside timer; not geometric work or a replacement benchmark\"}}"
    );
}
