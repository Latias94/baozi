use super::*;
use crate::LocalAxes;
use crate::canonical_scene::store::production::{CameraLocalId, LightLocalId};
use crate::canonical_scene::store::schema::{
    NODE_SCENE_EVIDENCE_FIELD_COUNT, SCENE_SCHEMA_REVISION, node_scene_evidence_field,
};
use crate::entity::{CameraProjection, LightDefinition, LightEmission, LightKind};
use crate::math::Color;

use super::super::test_support::SpatialBuildFault;

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
            Some(SCENE_SCHEMA_REVISION)
        );
        assert_released(failure, &account);
        assert!(!account.snapshot().poisoned(), "{fault:?}");
    }
}
