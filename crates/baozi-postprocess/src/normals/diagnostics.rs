//! Manual attribution only: the kernel without operation accounting is not a Scene operation.

use std::{convert::Infallible, hint::black_box, time::Instant};

use super::{Input, Mode, Output, PositionSource, VertexIds, Workspace, generate_into};
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
    let mut calls = 0_u64;
    for iteration in 0..23 {
        calls = 0;
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
                face_crosses: Some(&mut crosses),
            },
            || {
                calls += 1;
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
        "{{\"schema\":\"baozi-normal-kernel-diagnostic/v1\",\"position_source\":\"{label}\",\"faces\":{},\"control_calls\":{calls},\"warmups\":3,\"samples_ns\":{samples:?},\"scope\":\"kernel only, preallocated workspace, local counting observer, no operation accounting or publication\"}}",
        groups.len()
    );
}

#[test]
#[ignore = "manual optimized operation attribution; empty actions are not normal generation"]
fn observed_unit_protocol_with_empty_actions_preserves_exact_work_evidence() {
    use baozi_core::__scene_integration::integration::{
        SceneEditBootstrap, try_start_scene_edit_operation,
    };
    use baozi_operation::__retained_store_integration::authority::PreparedStoreEvidenceOnlyOutcome;

    use crate::operation::{PostProcessOwner, runtime};

    const ACTIONS: u64 = 982_943;
    let (scene, source_evidence) = crate::test_fixtures::canonical_triangle_import().into_parts();
    let mut samples = Vec::with_capacity(20);
    for iteration in 0..23 {
        let (writer_limits, _, _) = SceneEditBootstrap::try_new()
            .expect("original scene edit bootstrap")
            .into_parts();
        let (keys, ledger, start) =
            runtime::plan_selected_runtime(writer_limits, crate::PostProcessBudgets::default())
                .expect("original scene edit policy");
        let mut operation = try_start_scene_edit_operation::<PostProcessOwner, _, _>(
            &scene,
            crate::PostProcessControl::default().into_runtime(),
            ledger,
            start,
        )
        .expect("original scene edit operation");
        let initial = operation
            .producer_host()
            .dimension_state(keys.work)
            .expect("initial work state");
        let start = Instant::now();
        for _ in 0..ACTIONS {
            runtime::run_unit(&mut operation, keys, || black_box(()))
                .expect("one physically completed empty action");
        }
        let elapsed = start.elapsed().as_nanos();
        let state = operation
            .producer_host()
            .dimension_state(keys.work)
            .expect("completed work state");
        assert_eq!(state.attempted(), initial.attempted() + ACTIONS);
        assert_eq!(state.consumed(), initial.consumed() + ACTIONS);
        let prepared = operation
            .prepare_evidence_only_success(runtime::final_progress())
            .expect("every observed unit resolved before publication");
        let PreparedStoreEvidenceOnlyOutcome::Success(prepared) = prepared else {
            panic!("the unchanged empty-action operation must publish success");
        };
        let evidence = prepared.handoff().ledger_evidence;
        assert_eq!(evidence.header().current_live_bytes(), 0);
        assert_eq!(evidence.header().active_reservations(), 0);
        let terminal = evidence.dimension(keys.work).expect("final ordinary work");
        assert_eq!(terminal.attempted(), state.attempted());
        assert_eq!(terminal.consumed(), state.consumed());
        if iteration >= 3 {
            samples.push(elapsed);
        }
    }
    drop(source_evidence);
    drop(scene);
    println!(
        "{{\"schema\":\"baozi-observed-unit-diagnostic/v1\",\"actions\":{ACTIONS},\"warmups\":3,\"samples_ns\":{samples:?},\"scope\":\"actual scene edit operation run_unit with empty actions; setup and final publication outside timer; not geometric work or a replacement benchmark\"}}"
    );
}
