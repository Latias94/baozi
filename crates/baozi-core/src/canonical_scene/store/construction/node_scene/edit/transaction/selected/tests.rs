//! Exercise shared request composition and dependency refusal through the real core transaction.

use super::*;
use crate::attribute::{AttributeDomain, AttributeSemantic};
use crate::canonical_scene::store::construction::node_scene::materialization::node_scene_activation_roles;
use crate::canonical_scene::store::construction::node_scene::tests::{
    profile_fixture, single_trs_batch, successful_parts, test_account, test_factory, test_limits,
};
use crate::canonical_scene::store::construction::node_scene::{
    NodeSceneSuccessParts, NodeSceneTestTransaction,
};
use crate::canonical_scene::store::construction::{
    SceneConstructionPhase, SceneConstructionUnit, SceneFailureEvidenceWork, SceneStoreOperation,
    final_progress, try_plan_scene_operation,
};
use crate::canonical_scene::store::production::*;
use crate::deform::MorphDeltaSemantic;
use crate::geometry::TopologyKind;
use crate::identity::SceneGenerationId;
use baozi_operation::__retained_store_integration::activation::{
    CanonicalActivationOutcome, CanonicalActivationSeal, CanonicalCentralValidationOutcome,
    CanonicalFreezePermitIssue, CanonicalSemanticWorkError, CanonicalValidationProgress,
    StoreNeverActivatedCause,
};

fn source(
    account: &baozi_operation::PhysicalStorageAccount,
    directions: bool,
    tangent_morph: bool,
) -> NodeSceneSuccessParts {
    let positions = [
        0.0_f64, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0,
    ];
    let normals = [0.0_f64, 0.0, 1.0].repeat(3);
    let tangents = [1.0_f64, 0.0, 0.0, 1.0].repeat(3);
    let mut attributes = vec![GeometryAttributeAuthoring::f64(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        if directions {
            &positions[..9]
        } else {
            &positions
        },
    )];
    let primitives = if directions {
        attributes.push(GeometryAttributeAuthoring::f64(
            AttributeSemantic::Normal,
            AttributeDomain::Vertex,
            3,
            &normals,
        ));
        attributes.push(GeometryAttributeAuthoring::f64(
            AttributeSemantic::Tangent,
            AttributeDomain::Vertex,
            4,
            &tangents,
        ));
        vec![GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Absent,
            crate::MaterialAssignment::Unspecified,
            AuthoringRange::new(0, 3),
        )]
    } else {
        attributes.push(GeometryAttributeAuthoring::f64(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &positions,
        ));
        vec![
            GeometryPrimitiveAuthoring::new(
                GeometryTopologyAuthoring::TriangleStrip,
                Presence::Absent,
                crate::MaterialAssignment::Unspecified,
                AuthoringRange::new(0, 1),
            ),
            GeometryPrimitiveAuthoring::new(
                GeometryTopologyAuthoring::TriangleFan,
                Presence::Absent,
                crate::MaterialAssignment::Unspecified,
                AuthoringRange::new(1, 1),
            ),
            GeometryPrimitiveAuthoring::new(
                GeometryTopologyAuthoring::TriangleList,
                Presence::Absent,
                crate::MaterialAssignment::Unspecified,
                AuthoringRange::new(2, 0),
            ),
        ]
    };
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(
        0,
        primitives.len() as u32,
    ))];
    let geometry = GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]);
    let definitions = [MorphTargetDefinitionAuthoring::new(
        MorphTargetIdentity::new(0),
        Presence::Absent,
    )];
    let logical = [LogicalMorphAuthoring::new(
        AuthoringRange::new(0, u32::from(tangent_morph)),
        if tangent_morph { &[0.0] } else { &[] },
    )];
    let deltas = [MorphDeltaAuthoring::new(
        MorphDeltaSemantic::Tangent,
        MorphDeltaPayloadAuthoring::Sparse {
            logical_count: 3,
            indices: &[1],
            values: &[0.0, 0.25, 0.0],
        },
    )];
    let targets = [PrimitiveMorphTargetAuthoring::new(&deltas)];
    let deformations: Vec<_> = (0..primitives.len())
        .map(|_| {
            PrimitiveDeformationAuthoring::new(
                Presence::Absent,
                if tangent_morph { &targets } else { &[] },
            )
        })
        .collect();
    let deformation = DeformationAuthoringBatch::new(
        &[],
        if tangent_morph { &definitions } else { &[] },
        &logical,
        &deformations,
        &[],
    );
    let spatial = SpatialAuthoringBatch::new(&crate::SceneSpace::STANDARD_TARGET, &[], &[]);
    let appearance = AppearanceAuthoringBatch::empty();
    let animation = AnimationAuthoringBatch::empty();
    let profile = profile_fixture(
        spatial,
        single_trs_batch(),
        appearance,
        geometry,
        deformation,
        animation,
    );
    let input = SceneAuthoringBatch::new(
        spatial,
        single_trs_batch(),
        appearance,
        geometry,
        deformation,
        animation,
        profile.batch(),
    );
    let mut invocation_root_1 =
        crate::canonical_scene::store::construction::SceneCanonicalInvocationHost::new();
    let draft = NodeSceneTestTransaction::with_account(
        &mut invocation_root_1,
        account,
        test_limits(),
        input,
        SceneValidationLimits::default(),
    )
    .unwrap()
    .finish_draft()
    .unwrap();
    successful_parts(
        draft
            .publish_with_generation_source_for_test(SceneGenerationId::reserve)
            .unwrap(),
    )
}

struct FailAfter(u64);
impl TypedStorageBuildControl for FailAfter {
    type Error = StoreRuntimeError;
    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        if self.0 == 0 {
            return Err(StoreRuntimeError::ConstructionInvariantUnverified);
        }
        self.0 -= 1;
        Ok(action())
    }
    fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, Self::Error> {
        self.run_unit(action)
    }
}

/// Drives only the existing production transaction/validation/handoff APIs. Secondary plans share
/// the input requests, but their separately charged scratch owners cannot alter the sealed A plan.
fn run(
    scene: &ValidatedScene,
    edits: &[SceneGeometryReplacement<'_>],
    replan: bool,
    expected_rejection: Option<bool>,
) -> Option<(ValidatedScene, SceneEditEvidence)> {
    let bootstrap = SceneEditBootstrap::try_new().unwrap();
    let (writer, failure, admission) = bootstrap.into_parts();
    let fields = SceneEditBootstrap::selected_evidence_field_work(
        corner_count(edits) as u32,
        face_count(edits) as u32,
    )
    .unwrap();
    let start = try_plan_scene_operation(
        test_limits(),
        OperationControl::default(),
        256,
        SceneFailureEvidenceWork {
            max_evidence_members: 4,
            construction_work_limit: SceneEditBootstrap::failure_field_work(),
            validation_work_limit: fields,
        },
    )
    .unwrap();
    let mut invocation_root_2 =
        crate::canonical_scene::store::construction::SceneCanonicalInvocationHost::<
            SceneEditOutput,
            SceneEditValidationProof,
            SceneEditRejection,
            SceneEditRejection,
            super::super::SceneEditGenerationRejection,
            super::super::SceneEditEvidenceTag,
        >::new();
    let mut inputs = Some((start, failure, admission));
    let finish = invocation_root_2
        .run_body(|parts, disposition| {
            let (start, failure, admission) = inputs.take().unwrap();
            let mut operation = SceneStoreOperation::try_start_derived(
                parts.startup,
                start.control,
                start.ledger_plan,
                &scene.as_node_scene().canonical,
                start.start_plan,
            )
            .unwrap();
            let mut mappings = operation
                .with_action_execution(start.keys.dimension, |runtime| {
                    SceneGeometryPlanStorage::try_new(
                        runtime,
                        start.keys.phase,
                        start.keys.dimension,
                        edits.len(),
                    )
                })
                .unwrap();
            let mut secondary = operation
                .with_action_execution(start.keys.dimension, |runtime| {
                    SceneGeometryPlanStorage::try_new(
                        runtime,
                        start.keys.phase,
                        start.keys.dimension,
                        usize::from(replan),
                    )
                })
                .unwrap();
            let mut rejected = operation
                .with_action_execution(start.keys.dimension, |runtime| {
                    SceneGeometryPlanStorage::try_new(
                        runtime,
                        start.keys.phase,
                        start.keys.dimension,
                        usize::from(replan),
                    )
                })
                .unwrap();
            let owner = operation
                .into_session_outcome_owner(
                    node_scene_activation_roles(start.keys).session_policy(),
                    writer,
                    failure,
                )
                .unwrap();
            let mut transaction = admission.into_geometry_transaction(
                scene,
                edits,
                mappings.scratch(),
                OptionsDigest::sha256([91; 32]),
                SceneValidationLimits::default(),
            );
            let activated = owner
                .plan_and_activate_scoped(parts.output, parts.activation, |mut planning| {
                    let (sealed, completion) = match planning.with_semantic_work(|semantic| {
                        let sealed = transaction.preflight(semantic)?;
                        if replan {
                            let EditStoragePlan::Selection(a) = transaction.storage.unwrap() else {
                                panic!("selected A plan")
                            };
                            let before = [target_words(a, 0), target_words(a, 1)];
                            let b = GeometrySelectionPlan::try_new(
                                scene.as_node_scene().read_geometry(),
                                &edits[1..2],
                                secondary.scratch(),
                                semantic,
                            )
                            .unwrap();
                            assert_eq!(target_words(b, 0)[11], 1);
                            assert_eq!(target_words(b, 0)[12], 18);
                            // Initialization, row lookup, two triangles and the private mapping write complete;
                            // the next writer action rejects while A retains its already sealed mapping.
                            assert!(
                                GeometrySelectionPlan::try_new(
                                    scene.as_node_scene().read_geometry(),
                                    &edits[1..2],
                                    rejected.scratch(),
                                    &mut FailAfter(5)
                                )
                                .is_err()
                            );
                            assert_eq!(target_words(a, 0), before[0]);
                            assert_eq!(target_words(a, 1), before[1]);
                            assert_eq!(before[1][11], 7);
                            assert_eq!(before[1][12], 39);
                        }
                        Ok(sealed)
                    }) {
                        Ok(ready) => ready,
                        Err(CanonicalSemanticWorkError::Rejected(cause)) => {
                            return planning.reject(cause, 0x5534_0002);
                        }
                        Err(CanonicalSemanticWorkError::Runtime(cause)) => {
                            panic!("selected semantic work failed: {cause:?}")
                        }
                    };
                    match planning.seal_activation(sealed, completion) {
                        CanonicalActivationSeal::Sealed(permit) => {
                            permit.activate(|lane, plan| plan.materialize(lane))
                        }
                        CanonicalActivationSeal::Unsealed(failure) => failure.finish(),
                    }
                })
                .unwrap();
            let postflight = transaction.finish();
            let activated = match activated {
                CanonicalActivationOutcome::Activated(value) => {
                    assert!(expected_rejection.is_none());
                    value
                }
                CanonicalActivationOutcome::NeverActivated(value) => {
                    let StoreNeverActivatedCause::Requested(cause) = value.cause() else {
                        panic!("typed core rejection")
                    };
                    match expected_rejection {
                        Some(false) => {
                            assert!(matches!(cause, SceneEditRejection::TangentDependency))
                        }
                        Some(true) => assert!(matches!(
                            cause,
                            SceneEditRejection::MorphDependency {
                                semantic: MorphDeltaSemantic::Tangent
                            }
                        )),
                        None => panic!("unexpected rejection: {cause:?}"),
                    }
                    drop(mappings);
                    drop(secondary);
                    drop(rejected);
                    value.request_finish(disposition).unwrap();
                    return;
                }
                _ => panic!("selected core activation must resolve"),
            };
            let central = activated
                .validate_centrally(
                    parts.proof,
                    parts.validation,
                    CanonicalValidationProgress::new(
                        SceneConstructionPhase::Validation,
                        SceneConstructionUnit::WorkUnits,
                    ),
                    |lane, output| postflight.build_evidence(lane, output),
                    |planning, output| postflight.validate_candidate(planning, output),
                )
                .unwrap();
            drop(mappings);
            drop(secondary);
            drop(rejected);
            let CanonicalCentralValidationOutcome::Candidate(candidate) = central else {
                panic!("centrally validated selected candidate")
            };
            candidate.request_finish(disposition).unwrap();
        })
        .unwrap();
    let completion = finish.resume();
    let candidate = match completion.outcome {
        baozi_operation::__retained_store_integration::activation::CanonicalInvocationOutcome::Candidate(candidate) => candidate,
        baozi_operation::__retained_store_integration::activation::CanonicalInvocationOutcome::NeverActivated(rejected) => {
            let (fallback, _) = rejected.prepare_minimal_fallback().unwrap().into_parts();
            drop(fallback.handoff());
            return None;
        }
        _ => panic!("selected edit must restore its actual disposition"),
    };
    let CanonicalFreezePermitIssue::Ready(permit) = candidate
        .try_issue_freeze_permit(completion.generation.interruption, final_progress())
        .unwrap()
    else {
        panic!("selected freeze")
    };
    let SceneEditPublication::Success(published) =
        consume_scene_edit_freeze_permit(permit, completion.generation.rejection).unwrap()
    else {
        panic!("selected success")
    };
    let published = published.into_parts();
    Some((published.0, published.1))
}

#[test]
fn shared_strip_fan_requests_survive_successful_and_rejected_secondary_plans_through_publication() {
    let factory = test_factory();
    let account = test_account(&factory, 2048);
    let source = source(&account, false, false);
    let scene = ValidatedScene::from_node_scene(source.scene);
    let ids: Vec<_> = scene
        .meshes()
        .flat_map(|m| m.primitives().map(|p| p.id()))
        .collect();
    let strip = [[0, 1, 2], [1, 3, 2]];
    let fan = [[1, 2, 0], [2, 3, 0]];
    let mut edits = [
        SceneGeometryReplacement::new(ids[0]),
        SceneGeometryReplacement::new(ids[1]),
    ];
    edits[0].origins = Some(SceneTriangleOrigins {
        triangles: &strip,
        face_offsets: &[0, 1, 2],
    });
    edits[1].origins = Some(SceneTriangleOrigins {
        triangles: &fan,
        face_offsets: &[0, 1, 2],
    });
    let generation = scene.generation();
    let (output, evidence) = run(&scene, &edits, true, None).unwrap();
    let SceneEditEvidence::Selection(evidence) = evidence else {
        panic!("selected evidence")
    };
    assert_ne!(output.generation(), generation);
    for (index, expected) in [strip, fan].iter().enumerate() {
        let target = evidence.target(index).unwrap();
        assert_eq!(target.input_target(), ids[index]);
        assert_eq!(target.output_target().generation(), output.generation());
        assert_eq!(target.corner_origins().unwrap(), expected.as_flattened());
        assert_eq!(target.face_offsets().unwrap(), [0, 1, 2]);
        assert_eq!(
            output
                .primitive(target.output_target())
                .unwrap()
                .indices()
                .unwrap(),
            expected.as_flattened()
        );
    }
    drop(output);
    drop(scene);
    drop(source.report);
    assert_eq!(
        evidence.target(1).unwrap().corner_origins().unwrap(),
        fan.as_flattened()
    );
    drop(evidence);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn common_normal_change_refuses_preserved_tangent_and_its_morph_before_publication() {
    for morph in [false, true] {
        let factory = test_factory();
        let account = test_account(&factory, 2048);
        let source = source(&account, true, morph);
        let scene = ValidatedScene::from_node_scene(source.scene);
        let target = scene
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap()
            .id();
        let generation = scene.generation();
        let geometry = scene.as_node_scene().read_geometry();
        let bits: Vec<_> = geometry.f64_values.iter().map(|v| v.to_bits()).collect();
        let morph_bits: Vec<_> = scene
            .as_node_scene()
            .read_deformation()
            .morph_values
            .iter()
            .map(|v| v.to_bits())
            .collect();
        let physical = account.snapshot();
        let mut edits = [SceneGeometryReplacement::new(target)];
        edits[0].normals = Some(VectorValues::F64(&[[1.0, 0.0, 0.0]; 3]));
        assert!(run(&scene, &edits, false, Some(morph)).is_none());
        assert_eq!(scene.generation(), generation);
        assert_eq!(
            scene
                .as_node_scene()
                .read_geometry()
                .f64_values
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            bits
        );
        assert_eq!(
            scene
                .as_node_scene()
                .read_deformation()
                .morph_values
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            morph_bits
        );
        assert!(std::ptr::eq(
            geometry,
            scene.as_node_scene().read_geometry()
        ));
        assert_eq!(
            account.snapshot().live_retained_bytes(),
            physical.live_retained_bytes()
        );
        assert_eq!(
            account.snapshot().live_retained_allocations(),
            physical.live_retained_allocations()
        );
        assert!(!account.snapshot().poisoned());
        drop(scene);
        drop(source.report);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
    }
}

#[test]
fn selected_unchanged_fan_and_empty_list_keep_source_lineage_and_final_identity() {
    let factory = test_factory();
    let account = test_account(&factory, 2048);
    let source = source(&account, false, false);
    let scene = ValidatedScene::from_node_scene(source.scene);
    let ids: Vec<_> = scene
        .meshes()
        .flat_map(|m| m.primitives().map(|p| p.id()))
        .collect();
    let mut edits: Vec<_> = ids
        .iter()
        .copied()
        .map(SceneGeometryReplacement::new)
        .collect();
    edits[0].origins = Some(SceneTriangleOrigins {
        triangles: &[[0, 1, 2], [1, 3, 2]],
        face_offsets: &[0, 1, 2],
    });
    let (output, evidence) = run(&scene, &edits, false, None).unwrap();
    let SceneEditEvidence::Selection(evidence) = evidence else {
        panic!("selected evidence")
    };
    assert_eq!(evidence.len(), 3);
    assert!(evidence.target(0).unwrap().changed());
    for (index, topology) in [
        (1, TopologyKind::TriangleFan),
        (2, TopologyKind::TriangleList),
    ] {
        let target = evidence.target(index).unwrap();
        assert!(!target.changed());
        assert_eq!(target.input_target(), ids[index]);
        assert_eq!(target.output_target().generation(), output.generation());
        assert!(target.corner_origins().is_none());
        let primitive = output.primitive(target.output_target()).unwrap();
        assert_eq!(primitive.topology(), topology);
        for parent in output.owner_provenance(primitive.metadata_owner()).unwrap() {
            assert!(matches!(parent.kind(), crate::ProvenanceKindRef::Source(_)));
        }
    }
    drop(output);
    drop(evidence);
    drop(scene);
    drop(source.report);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert!(!account.snapshot().poisoned());
}
