use super::*;
use crate::LocalAxes;
use crate::canonical_scene::store::production::{CameraLocalId, LightLocalId};
use crate::canonical_scene::store::schema::{
    NODE_SCENE_EVIDENCE_FIELD_COUNT, SCENE_DIRECTORY_FIELD_COUNT, node_scene_evidence_field,
    scene_directory_field,
};
use crate::entity::{CameraProjection, LightDefinition, LightEmission, LightKind};
use crate::math::Color;

use super::super::test_support::SpatialBuildFault;

#[test]
fn directory_and_evidence_revisions_gate_scene_publication() {
    let factory = test_factory();
    let account = test_account(&factory, 256);
    for revision in [1, 0, 8, u32::MAX] {
        for (site, directory_revision, evidence_revision) in [
            ("directory", Some(revision), None),
            ("evidence", None, Some(revision)),
        ] {
            let generation_source = AtomicU64::new(0);
            let build = finish_node_batch_with_faults(
                &account,
                test_limits(),
                single_trs_batch(),
                SceneValidationLimits::default(),
                NodeSceneBuildFaults {
                    directory_revision,
                    evidence_revision,
                    ..NodeSceneBuildFaults::none()
                },
            );
            let terminal = build
                .publish_with_generation_source_for_test(|| {
                    SceneGenerationId::reserve_from(&generation_source)
                })
                .unwrap_or_else(|_| panic!("{site} revision {revision}: bounded terminal"));
            if revision == 1 {
                let parts = successful_parts(terminal);
                let directory = parts.scene.canonical.root();
                assert_eq!(
                    (directory.rows(), directory.fields()),
                    (1, SCENE_DIRECTORY_FIELD_COUNT)
                );
                assert_eq!(
                    directory.scalar::<u32>(0, scene_directory_field::SCHEMA_REVISION),
                    Some(1)
                );
                assert_eq!(
                    parts
                        .report
                        .outcome_evidence
                        .root()
                        .scalar::<u32>(0, node_scene_evidence_field::SCHEMA_REVISION),
                    Some(1)
                );
                assert_eq!(generation_source.load(Ordering::Relaxed), 1);
                drop(parts);
            } else {
                assert_eq!(
                    generation_source.load(Ordering::Relaxed),
                    0,
                    "{site} {revision}"
                );
                let ledger = match &terminal {
                    NodeSceneConstructionTerminal::RichFailure(failure) if site == "directory" => {
                        // The invalid directory is discarded; separately owned
                        // current-revision outcome evidence remains trustworthy.
                        assert_eq!(
                            terminal_failure_code(failure.ledger_evidence()),
                            NODE_SCENE_BUILD_INVARIANT_CODE
                        );
                        let evidence = failure.outcome_evidence.outcome_evidence();
                        let root = evidence.root();
                        assert_eq!(
                            (root.rows(), root.fields()),
                            (1, NODE_SCENE_EVIDENCE_FIELD_COUNT)
                        );
                        assert_eq!(
                            root.scalar::<u32>(0, node_scene_evidence_field::SCHEMA_REVISION),
                            Some(1)
                        );
                        failure.ledger_evidence()
                    }
                    NodeSceneConstructionTerminal::MinimalFallback(fallback)
                        if site == "evidence" =>
                    {
                        // Invalid evidence itself cannot be handed off as a
                        // rich owner. Require the precise contract-failure path.
                        assert_eq!(
                            terminal_failure_class(fallback.ledger_evidence()),
                            StorePreparationFailureClass::ProductGraph
                        );
                        assert_eq!(
                            terminal_failure_code(fallback.ledger_evidence()),
                            0x4356_4346
                        );
                        fallback.ledger_evidence()
                    }
                    _ => panic!("{site} revision {revision}: wrong failure owner"),
                };
                assert_eq!(ledger.header().schema_revision(), 1);
                assert_eq!(ledger.header().current_live_bytes(), 0);
                assert_eq!(ledger.header().active_reservations(), 0);
                assert_eq!(
                    ledger
                        .dimension_by_semantic(STORE_DIMENSION)
                        .unwrap()
                        .committed_canonical_output(),
                    0
                );
                drop(terminal);
            }
            assert_eq!(
                account.snapshot().live_retained_bytes(),
                0,
                "{site} {revision}"
            );
            assert_eq!(
                account.snapshot().live_retained_allocations(),
                0,
                "{site} {revision}"
            );
            assert!(!account.snapshot().poisoned(), "{site} {revision}");
        }
    }
}

#[test]
fn central_validation_rejects_corrupted_spatial_storage_and_placement_closure() {
    let factory = test_factory();
    let account = test_account(&factory, 256);
    let generation_source = AtomicU64::new(0);
    for fault in SpatialBuildFault::ALL {
        let roots = [NodeLocalId::new(0)];
        let nodes = [NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Present(CameraLocalId::new(0)),
            Presence::Present(LightLocalId::new(0)),
        )];
        let cameras =
            [CameraProjection::perspective(LocalAxes::STANDARD, 1.0, None, 0.125, None).unwrap()];
        let lights = [LightDefinition::new(
            LightKind::spot(Some(32.0), 0.125, 0.75).unwrap(),
            LocalAxes::STANDARD,
            LightEmission::try_from_scaled_linear_srgb(Color::WHITE, 2.0).unwrap(),
        )];
        let spatial = SpatialAuthoringBatch::new(&SceneSpace::STANDARD_TARGET, &cameras, &lights);
        let nodes = NodeAuthoringBatch::new(Presence::Present(&roots), &nodes);
        let appearance = AppearanceAuthoringBatch::empty();
        let geometry = GeometryAuthoringBatch::empty();
        let deformation = DeformationAuthoringBatch::empty();
        let animations = AnimationAuthoringBatch::empty();
        let profile = profile_fixture(
            spatial,
            nodes,
            appearance,
            geometry,
            deformation,
            animations,
        );
        let input = SceneAuthoringBatch::new(
            spatial,
            nodes,
            appearance,
            geometry,
            deformation,
            animations,
            profile.batch(),
        );
        let build = NodeSceneTestTransaction::with_account(
            &account,
            test_limits(),
            input,
            SceneValidationLimits::default(),
        )
        .unwrap()
        .finish_draft_with_faults(NodeSceneBuildFaults::altered_spatial(fault))
        .unwrap_or_else(|_| {
            panic!("{fault:?}: corrupted spatial candidate must be rejected cleanly")
        });
        let terminal = build
            .publish_with_generation_source_for_test(|| {
                SceneGenerationId::reserve_from(&generation_source)
            })
            .unwrap_or_else(|_| {
                panic!("{fault:?}: central rejection must publish bounded evidence")
            });
        let failure = rich_failure(terminal);
        assert_eq!(generation_source.load(Ordering::Relaxed), 0, "{fault:?}");
        assert_eq!(
            terminal_failure_code(failure.ledger_evidence()),
            NODE_SCENE_BUILD_INVARIANT_CODE,
            "{fault:?}"
        );
        assert_eq!(
            failure.ledger_evidence().header().current_live_bytes(),
            0,
            "{fault:?}"
        );
        assert_eq!(
            failure.ledger_evidence().header().active_reservations(),
            0,
            "{fault:?}"
        );
        assert_eq!(
            failure
                .ledger_evidence()
                .dimension_by_semantic(STORE_DIMENSION)
                .unwrap()
                .committed_canonical_output(),
            0,
            "{fault:?}"
        );
        let evidence = failure.outcome_evidence.outcome_evidence();
        let root = evidence.root();
        assert_eq!(
            (root.rows(), root.fields()),
            (1, NODE_SCENE_EVIDENCE_FIELD_COUNT)
        );
        assert_eq!(
            root.scalar::<u32>(0, node_scene_evidence_field::SCHEMA_REVISION),
            Some(1)
        );
        assert_released(failure, &account);
        assert!(!account.snapshot().poisoned(), "{fault:?}");
    }
}
