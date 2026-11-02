//! Central validation proves the actual candidate and every rooted remap scalar together.

use baozi_operation::__retained_store_integration::activation::{
    CanonicalValidationDecision, CanonicalValidationPlanning, CanonicalValidationSession,
    FailureEvidenceMember, FailureEvidenceValidationSession, OutcomeEvidenceValidationMember,
};
use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit, RecordBlock, StoreWorkError};

use crate::canonical_scene::store::construction::node_scene::scratch::ScopedNodeSceneValidationScratchOwner;
use crate::canonical_scene::store::schema::{NodeSceneOutcomeEvidenceSchema, SceneDirectorySchema};
use crate::canonical_scene::store::validation::{
    NodeSceneValidationProof, validate_node_scene_canonical_graph_metered,
};
use crate::identity::SceneGenerationId;

use super::evidence::SceneEditEvidenceRecipe;
use super::{
    SceneEditEvidenceTag, SceneEditFailureEvidence, SceneEditOutput, SceneEditPostflight,
    SceneEditRejection, SceneGeometryCornerOriginsTag, SceneGeometryFaceOffsetsTag, schema,
};

/// Proof of the actual canonical graph, scalar correspondence, and rooted edit evidence.
#[derive(Clone, Copy)]
pub struct SceneEditValidationProof {
    pub(super) scene: NodeSceneValidationProof,
    pub(super) source_generation: SceneGenerationId,
    pub(super) input_target: super::SceneEditTarget,
}

impl SceneEditPostflight<'_> {
    #[allow(clippy::type_complexity)]
    pub fn validate_candidate<'session, 'storage, 'scope, 'operation, O, PP, U>(
        &self,
        planning: CanonicalValidationPlanning<
            'session,
            'storage,
            'scope,
            'operation,
            O,
            PP,
            U,
            RecordBlock,
            SceneDirectorySchema,
            RecordBlock,
            SceneEditEvidenceTag,
            SceneEditValidationProof,
            SceneEditRejection,
        >,
        output: &SceneEditOutput,
    ) -> Result<(), StoreWorkError>
    where
        O: OperationOwner,
        PP: ProgressPhase,
        U: ProgressUnit,
    {
        let scene = output.scene.inner();
        let plan = scene.central_validation;
        let scratch_plan = plan.scratch_plan();
        let recipe = self.candidate_evidence(output);
        planning.validate_with(
            |checkpoint| {
                ScopedNodeSceneValidationScratchOwner::try_allocate(checkpoint, scratch_plan)
            },
            |session, directory, evidence, scratch| {
                let proof = if scratch.initialize(session, scratch_plan)?
                    && scene.draft.brand == self.brand
                    && output.source_generation == self.input.generation()
                {
                    if let Some(storage) = self.storage {
                        let expected = storage.facts(
                            self.input.as_node_scene(),
                            self.replacement,
                            scene.evidence_counts,
                        );
                        if Some(output.facts) == expected {
                            if let Some(counts) =
                                validate_changed_root(session, evidence, &recipe, self.storage)?
                            {
                                if storage.validate_correspondence(&scene.tables, session)? {
                                    let mut scratch = scratch.metered_view();
                                    validate_node_scene_canonical_graph_metered(
                                        session,
                                        directory,
                                        counts,
                                        scene.evidence_counts,
                                        scene.refs,
                                        &scene.tables,
                                        plan,
                                        &mut scratch,
                                    )?
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };
                Ok(match proof {
                    Some(scene) => CanonicalValidationDecision::accept(SceneEditValidationProof {
                        scene,
                        source_generation: output.source_generation,
                        input_target: self.replacement.target(),
                    }),
                    None => CanonicalValidationDecision::reject(
                        SceneEditRejection::construction_invariant(),
                        super::super::super::NODE_SCENE_BUILD_INVARIANT_CODE,
                    ),
                })
            },
        )
    }
}

fn validate_changed_root<'session, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, '_, '_, '_, O, PP, U>,
    root: OutcomeEvidenceValidationMember<'session, RecordBlock, SceneEditEvidenceTag>,
    recipe: &SceneEditEvidenceRecipe<'_>,
    storage: Option<super::EditStoragePlan<'_>>,
) -> Result<
    Option<OutcomeEvidenceValidationMember<'session, RecordBlock, NodeSceneOutcomeEvidenceSchema>>,
    StoreWorkError,
>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let (mut counts, mut corners, mut faces) = (None, None, None);
    let complete =
        session.visit_outcome_evidence_records(root, 1, schema::ROOT_FIELDS, |_, row| {
            let mut valid = true;
            for (field, expected) in [
                schema::REVISION,
                schema::OUTCOME_CHANGED,
                schema::OPERATION_SEMANTIC_VERSION,
            ]
            .into_iter()
            .chain(recipe.facts.words)
            .enumerate()
            {
                valid &= row.scalar(field, |value: u32| value)? == Some(expected);
            }
            for (word, expected) in recipe.facts.digest.into_iter().enumerate() {
                valid &= row.scalar(schema::field::OPTIONS_DIGEST_FIRST + word, |value: u64| {
                    value
                })? == Some(expected);
            }
            for (offset, expected) in recipe.facts.units.into_iter().enumerate() {
                valid &= row
                    .scalar(schema::field::SOURCE_UNIT_BITS + offset, |value: u64| value)?
                    == Some(expected);
            }
            counts = row.record_ref::<NodeSceneOutcomeEvidenceSchema, _>(
                schema::field::SCENE_COUNTS,
                |member| member,
            )?;
            if !matches!(
                recipe.facts.words[0],
                schema::OPERATION_TRIANGULATE | schema::OPERATION_SELECTION
            ) {
                valid &= row.scalar(schema::field::CORNER_ORIGINS, |value: u8| value)? == Some(0);
                valid &= row.scalar(schema::field::FACE_OFFSETS, |value: u8| value)? == Some(0);
            } else {
                corners = row.sequence_ref::<u32, SceneGeometryCornerOriginsTag, _>(
                    schema::field::CORNER_ORIGINS,
                    |member| member,
                )?;
                faces = row.sequence_ref::<u32, SceneGeometryFaceOffsetsTag, _>(
                    schema::field::FACE_OFFSETS,
                    |member| member,
                )?;
            }
            Ok(valid)
        })?;
    if !matches!(
        recipe.facts.words[0],
        schema::OPERATION_TRIANGULATE | schema::OPERATION_SELECTION
    ) {
        return Ok(if complete { counts } else { None });
    }
    let (Some(counts), Some(corners), Some(faces)) = (counts, corners, faces) else {
        return Ok(None);
    };
    if !complete {
        return Ok(None);
    }
    if let Some(super::EditStoragePlan::Selection(plan)) = storage {
        let triples = plan.edits().iter().flat_map(|e| {
            e.origins
                .into_iter()
                .flat_map(|o| o.triangles.iter().flat_map(|t| t.iter().copied()))
        });
        let mut expected_corners = std::iter::once(recipe.facts.words[7]).chain(triples);
        let Some(corner_fields) =
            (recipe.facts.words[7] as usize).checked_add(schema::SELECTION_CORNER_PREFIX_FIELDS)
        else {
            return Ok(None);
        };
        if !session.visit_outcome_evidence_sequence(corners, corner_fields, |_, actual| {
            expected_corners.next() == Some(actual)
        })? {
            return Ok(None);
        }
        let headers = (0..plan.edits().len()).flat_map(|i| super::selected::target_words(plan, i));
        let offsets = plan.edits().iter().flat_map(|e| {
            e.origins
                .into_iter()
                .flat_map(|o| o.face_offsets.iter().copied())
        });
        let mut expected = headers.chain(offsets);
        if !session.visit_outcome_evidence_sequence(
            faces,
            recipe.facts.words[8] as usize,
            |_, actual| expected.next() == Some(actual),
        )? {
            return Ok(None);
        }
    } else {
        if !session.visit_outcome_evidence_sequence(
            corners,
            recipe.facts.words[7] as usize,
            |index, actual| {
                recipe
                    .source_corner_triangles()
                    .get(index / 3)
                    .is_some_and(|triangle| triangle[index % 3] == actual)
            },
        )? {
            return Ok(None);
        }
        if !session.visit_outcome_evidence_sequence(
            faces,
            recipe.source_face_triangle_offsets().len(),
            |index, actual| {
                recipe.source_face_triangle_offsets().get(index).copied() == Some(actual)
            },
        )? {
            return Ok(None);
        }
    }
    Ok(Some(counts))
}

impl SceneEditFailureEvidence {
    /// Validates the fixed two-member preflight/materialization failure graph.
    pub fn validate_failure<'session, 'storage, 'control, O, PP, U>(
        &self,
        session: &mut FailureEvidenceValidationSession<'session, 'storage, 'control, O, PP, U>,
        root: FailureEvidenceMember<'session, RecordBlock, SceneEditEvidenceTag>,
    ) -> bool
    where
        O: OperationOwner,
        PP: ProgressPhase,
        U: ProgressUnit,
    {
        let mut counts = None;
        let valid = session
            .visit_records(root, |index, row| {
                let mut valid = index == 0 && row.fields() == schema::ROOT_FIELDS;
                for (field, expected) in [
                    schema::REVISION,
                    schema::OUTCOME_FAILED,
                    schema::OPERATION_SEMANTIC_VERSION,
                ]
                .into_iter()
                .chain([0; 13])
                .enumerate()
                {
                    valid &= row
                        .scalar::<u32, _>(field, |value| value == expected)
                        .unwrap_or(false);
                }
                for field in schema::field::OPTIONS_DIGEST_FIRST..schema::field::SCENE_COUNTS {
                    valid &= row
                        .scalar::<u64, _>(field, |value| value == 0)
                        .unwrap_or(false);
                }
                counts = row
                    .record_ref::<NodeSceneOutcomeEvidenceSchema, _>(
                        schema::field::SCENE_COUNTS,
                        |member| member,
                    )
                    .ok();
                valid &= row
                    .scalar::<u8, _>(schema::field::CORNER_ORIGINS, |value| value == 0)
                    .unwrap_or(false);
                valid &= row
                    .scalar::<u8, _>(schema::field::FACE_OFFSETS, |value| value == 0)
                    .unwrap_or(false);
                valid
            })
            .unwrap_or(false);
        valid && counts.is_some_and(|counts| self.scene_counts().validate_failure(session, counts))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use baozi_operation::__retained_store_integration::activation::{
        CanonicalActivationOutcome, CanonicalActivationSeal, CanonicalCentralValidationOutcome,
        CanonicalFreezePermitIssue, CanonicalValidationProgress, FailureEvidenceOutcome,
        RootedFailureValidation,
    };
    use baozi_operation::__retained_store_integration::authority::{
        PreparedStoreFailureOutcome, PreparedStoreSuccessOutcome,
    };
    use baozi_operation::OperationControl;

    use crate::SceneValidationLimits;
    use crate::canonical_scene::store::construction::node_scene::edit::tests::{
        build_single_trs_scene, try_build_single_trs_scene,
    };
    use crate::canonical_scene::store::construction::node_scene::materialization::node_scene_activation_roles;
    use crate::canonical_scene::store::construction::node_scene::snapshot::NodeSceneSnapshotLimits;
    use crate::canonical_scene::store::construction::node_scene::tests::{
        test_account, test_factory, test_limits,
    };
    use crate::canonical_scene::store::construction::{
        STORE_DIMENSION, SceneConstructionPhase, SceneConstructionUnit, SceneFailureEvidenceWork,
        SceneStoreOperation, final_progress, try_plan_scene_operation,
    };
    use crate::space_conversion::TargetSpace;

    use super::super::{
        SceneEditBootstrap, consume_scene_edit_freeze_permit, resolve_scene_space_conversion,
    };
    use super::*;

    #[test]
    fn changed_root_revisions_gate_scene_publication_and_preserve_input() {
        let factory = test_factory();
        let account = test_account(&factory, 1_024);
        // Source and destination must reserve from the same production lineage.
        let source =
            try_build_single_trs_scene(&account, crate::identity::SceneGenerationId::reserve)
                .expect("single-node edit fixture must publish success");
        let scene = crate::ValidatedScene::from_node_scene(source.scene);
        let limits = NodeSceneSnapshotLimits::try_new(0, 0, 100_000).unwrap();
        let before = scene
            .as_node_scene()
            .capture_snapshot(limits, None)
            .unwrap();
        let generation = scene.generation();
        let retained = account.snapshot();

        for revision in [1, 0, 4, u32::MAX] {
            let (writer, failure, admission) = SceneEditBootstrap::try_new().unwrap().into_parts();
            let start = try_plan_scene_operation(
                test_limits(),
                OperationControl::default(),
                256,
                SceneFailureEvidenceWork {
                    max_evidence_members: SceneEditBootstrap::success_evidence_members(),
                    construction_work_limit: SceneEditBootstrap::failure_field_work(),
                    validation_work_limit: SceneEditBootstrap::failure_field_work(),
                },
            )
            .unwrap();
            let operation = SceneStoreOperation::try_start_derived(
                start.control,
                start.ledger_plan,
                &scene.as_node_scene().canonical,
                start.start_plan,
            )
            .unwrap();
            let owner = operation
                .into_session_outcome_owner(
                    node_scene_activation_roles(start.keys).session_policy(),
                    writer,
                    failure,
                )
                .unwrap();
            let resolved = resolve_scene_space_conversion(
                &scene,
                TargetSpace::STANDARD,
                Some(TargetSpace::STANDARD),
            )
            .unwrap();
            assert!(!resolved.is_unchanged());
            let mut transaction =
                admission.into_space_transaction(resolved, SceneValidationLimits::default());
            let activated = owner
                .plan_and_activate_scoped(|mut planning| {
                    let mut semantic = planning.begin_semantic_work().unwrap();
                    let sealed = transaction.preflight(&mut semantic).unwrap();
                    let completion = semantic.finish_exact().unwrap();
                    match planning.seal_activation(sealed, completion) {
                        CanonicalActivationSeal::Sealed(permit) => {
                            permit.activate(|lane, plan| plan.materialize(lane))
                        }
                        CanonicalActivationSeal::Unsealed(failure) => failure.finish(),
                    }
                })
                .unwrap();
            let postflight = transaction.finish();
            let CanonicalActivationOutcome::Activated(activated) = activated else {
                panic!("revision {revision}: valid space edit must materialize")
            };
            let central = activated
                .validate_centrally(
                    CanonicalValidationProgress::new(
                        SceneConstructionPhase::Validation,
                        SceneConstructionUnit::WorkUnits,
                    ),
                    |lane, output| {
                        if revision == 1 {
                            return postflight.build_evidence(lane, output);
                        }
                        // Copy only this fixture's current root shape, changing its revision cell.
                        let recipe = postflight.candidate_evidence(output);
                        let counts = lane
                            .try_retain_records(
                                recipe.counts.record_plan(),
                                NodeSceneOutcomeEvidenceSchema,
                                |writer| writer.write_row(|row| recipe.counts.write_row(row)),
                            )
                            .unwrap();
                        let root = lane
                            .try_retain_records(
                                postflight.failure_evidence().root_record_plan(),
                                SceneEditEvidenceTag,
                                |writer| {
                                    writer.write_row(|row| {
                                        row.write_scalar(revision)?;
                                        row.write_scalar(schema::OUTCOME_CHANGED)?;
                                        row.write_scalar(schema::OPERATION_SEMANTIC_VERSION)?;
                                        for word in recipe.facts.words {
                                            row.write_scalar(word)?;
                                        }
                                        for word in recipe.facts.digest {
                                            row.write_scalar(word)?;
                                        }
                                        for unit in recipe.facts.units {
                                            row.write_scalar(unit)?;
                                        }
                                        row.write_record_ref(counts)?;
                                        row.write_scalar(0_u8)?;
                                        row.write_scalar(0_u8)
                                    })
                                },
                            )
                            .unwrap();
                        lane.complete(root)
                    },
                    |planning, output| postflight.validate_candidate(planning, output),
                )
                .unwrap();
            match central {
                CanonicalCentralValidationOutcome::Candidate(candidate) => {
                    assert_eq!(revision, 1);
                    let CanonicalFreezePermitIssue::Ready(permit) =
                        candidate.try_issue_freeze_permit(final_progress()).unwrap()
                    else {
                        panic!("v1 changed root must reach publication")
                    };
                    let published = consume_scene_edit_freeze_permit(
                        permit,
                        |_| panic!("v1 generation admission"),
                        |token, joint, seal| {
                            let PreparedStoreSuccessOutcome::Success(store) =
                                joint.prepare(seal).unwrap()
                            else {
                                panic!("v1 changed root must publish successfully")
                            };
                            token.handoff(store).unwrap().into_parts()
                        },
                    )
                    .unwrap();
                    assert_ne!(published.0.generation(), generation);
                    let snapshot = published
                        .0
                        .as_node_scene()
                        .capture_snapshot(limits, None)
                        .unwrap();
                    assert_eq!(snapshot.schema_revision, 1);
                    assert_ne!(snapshot.digest, before.digest);
                    drop(published);
                }
                CanonicalCentralValidationOutcome::Rejected(rejected) => {
                    assert_ne!(revision, 1);
                    let (prepared, _) = rejected.prepare_minimal_fallback().unwrap().into_parts();
                    let published = prepared.handoff();
                    assert_eq!(
                        published
                            .ledger_evidence
                            .dimension_by_semantic(STORE_DIMENSION)
                            .unwrap()
                            .committed_canonical_output(),
                        0
                    );
                    drop(published);
                }
                _ => panic!("revision {revision}: expected a semantic revision decision"),
            }
            assert_eq!(scene.generation(), generation);
            assert_eq!(
                scene
                    .as_node_scene()
                    .capture_snapshot(limits, None)
                    .unwrap()
                    .digest,
                before.digest
            );
            assert_eq!(
                account.snapshot().live_retained_bytes(),
                retained.live_retained_bytes()
            );
            assert_eq!(
                account.snapshot().live_retained_allocations(),
                retained.live_retained_allocations()
            );
            assert!(!account.snapshot().poisoned());
        }
        drop(scene);
        drop(source.report);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }

    #[test]
    fn failed_root_revisions_gate_rich_failure_and_preserve_input() {
        let factory = test_factory();
        let account = test_account(&factory, 1_024);
        let generation_source = AtomicU64::new(0);
        let source = build_single_trs_scene(&account, &generation_source);
        let scene = crate::ValidatedScene::from_node_scene(source.scene);
        let limits = NodeSceneSnapshotLimits::try_new(0, 0, 100_000).unwrap();
        let before = scene
            .as_node_scene()
            .capture_snapshot(limits, None)
            .unwrap();
        let generation = scene.generation();
        let retained = account.snapshot();

        for revision in [1, 0, 4, u32::MAX] {
            let (writer, failure, admission) = SceneEditBootstrap::try_new().unwrap().into_parts();
            let evidence = admission.failure_evidence();
            let start = try_plan_scene_operation(
                test_limits(),
                OperationControl::default(),
                256,
                SceneFailureEvidenceWork {
                    max_evidence_members: SceneEditBootstrap::success_evidence_members(),
                    construction_work_limit: SceneEditBootstrap::failure_field_work(),
                    validation_work_limit: SceneEditBootstrap::failure_field_work(),
                },
            )
            .unwrap();
            let operation = SceneStoreOperation::try_start_derived(
                start.control,
                start.ledger_plan,
                &scene.as_node_scene().canonical,
                start.start_plan,
            )
            .unwrap();
            let owner = operation
                .into_session_outcome_owner(
                    node_scene_activation_roles(start.keys).session_policy(),
                    writer,
                    failure,
                )
                .unwrap();
            let rejected = owner
                .reject_without_activation::<(), (), RecordBlock, SceneDirectorySchema>(
                    (),
                    0x5531_3201,
                )
                .unwrap();
            let CanonicalActivationOutcome::NeverActivated(rejected) = rejected else {
                panic!("fixed preflight rejection must not activate canonical storage")
            };
            let rooted = rejected
                .build_failure_evidence(|lane| {
                    let counts = lane
                        .try_retain_records(
                            evidence.scene_counts().record_plan(),
                            NodeSceneOutcomeEvidenceSchema,
                            |writer| writer.write_row(|row| evidence.scene_counts().write_row(row)),
                        )
                        .unwrap();
                    let root = lane
                        .try_retain_records(
                            evidence.root_record_plan(),
                            SceneEditEvidenceTag,
                            |writer| {
                                writer.write_row(|row| {
                                    if revision == 1 {
                                        return evidence.write_root_row(row, counts);
                                    }
                                    row.write_scalar(revision)?;
                                    row.write_scalar(schema::OUTCOME_FAILED)?;
                                    row.write_scalar(schema::OPERATION_SEMANTIC_VERSION)?;
                                    for _ in schema::field::OPERATION_KIND
                                        ..schema::field::OPTIONS_DIGEST_FIRST
                                    {
                                        row.write_scalar(0_u32)?;
                                    }
                                    for _ in schema::field::OPTIONS_DIGEST_FIRST
                                        ..schema::field::SCENE_COUNTS
                                    {
                                        row.write_scalar(0_u64)?;
                                    }
                                    row.write_record_ref(counts)?;
                                    row.write_scalar(0_u8)?;
                                    row.write_scalar(0_u8)
                                })
                            },
                        )
                        .unwrap();
                    lane.complete(root)
                })
                .unwrap();
            let FailureEvidenceOutcome::Rooted(rooted) = rooted else {
                panic!("revision {revision}: complete two-member failure graph")
            };
            assert_eq!(rooted.outcome_evidence_len(), 2);
            let validation = rooted
                .validate_rich_failure(|session, root| evidence.validate_failure(session, root))
                .unwrap();
            match validation {
                RootedFailureValidation::Valid(validated) => {
                    assert_eq!(revision, 1);
                    let (prepared, _) = validated
                        .try_prepare(final_progress())
                        .unwrap()
                        .into_parts();
                    let PreparedStoreFailureOutcome::RichFailure(prepared) = prepared else {
                        panic!("v1 failure root must preserve rich evidence")
                    };
                    let published = prepared.handoff();
                    let evidence = published.outcome_evidence.outcome_evidence();
                    let root = evidence.root();
                    assert_eq!((root.rows(), root.fields()), (1, 25));
                    assert_eq!(root.scalar::<u32>(0, schema::field::REVISION), Some(1));
                    assert_eq!(
                        published
                            .ledger_evidence
                            .dimension_by_semantic(STORE_DIMENSION)
                            .unwrap()
                            .committed_canonical_output(),
                        0
                    );
                    drop(published);
                }
                RootedFailureValidation::Rejected(rejected) => {
                    assert_ne!(revision, 1);
                    let (prepared, _) = rejected.prepare_minimal_fallback().unwrap().into_parts();
                    drop(prepared.handoff());
                }
                RootedFailureValidation::Fallback(_) => {
                    panic!("revision {revision}: expected a semantic revision decision")
                }
            }
            assert_eq!(scene.generation(), generation);
            assert_eq!(
                scene
                    .as_node_scene()
                    .capture_snapshot(limits, None)
                    .unwrap()
                    .digest,
                before.digest
            );
            assert_eq!(generation_source.load(Ordering::Relaxed), 1);
            assert_eq!(
                account.snapshot().live_retained_bytes(),
                retained.live_retained_bytes()
            );
            assert_eq!(
                account.snapshot().live_retained_allocations(),
                retained.live_retained_allocations()
            );
            assert!(!account.snapshot().poisoned());
        }
        drop(scene);
        drop(source.report);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }
}
