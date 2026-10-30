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
use crate::identity::{PrimitiveId, SceneGenerationId};

use super::evidence::GeometryEditFacts;
use super::{
    SceneGeometryCornerOriginsTag, SceneGeometryEditEvidenceRecipe, SceneGeometryEditEvidenceTag,
    SceneGeometryEditFailureEvidence, SceneGeometryEditOutput, SceneGeometryEditPostflight,
    SceneGeometryEditRejection, SceneGeometryFaceOffsetsTag, schema,
};

/// Proof of the actual canonical graph, scalar correspondence, and rooted edit evidence.
#[derive(Clone, Copy)]
pub struct SceneGeometryEditValidationProof {
    pub(super) scene: NodeSceneValidationProof,
    pub(super) source_generation: SceneGenerationId,
    pub(super) input_target: PrimitiveId,
}

impl SceneGeometryEditPostflight<'_> {
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
            SceneGeometryEditEvidenceTag,
            SceneGeometryEditValidationProof,
            SceneGeometryEditRejection,
        >,
        output: &SceneGeometryEditOutput,
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
                    if let Some(geometry) = self.geometry {
                        let expected = GeometryEditFacts::new(
                            self.input.as_node_scene(),
                            self.replacement,
                            &geometry,
                        );
                        if output.facts == expected {
                            if let Some(counts) = validate_changed_root(session, evidence, &recipe)?
                            {
                                if geometry.validate_correspondence(
                                    scene.tables.geometry.as_ref(),
                                    session,
                                )? {
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
                    Some(scene) => {
                        CanonicalValidationDecision::accept(SceneGeometryEditValidationProof {
                            scene,
                            source_generation: output.source_generation,
                            input_target: self.replacement.target(),
                        })
                    }
                    None => CanonicalValidationDecision::reject(
                        SceneGeometryEditRejection::construction_invariant(),
                        super::super::super::NODE_SCENE_BUILD_INVARIANT_CODE,
                    ),
                })
            },
        )
    }
}

fn validate_changed_root<'session, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, '_, '_, '_, O, PP, U>,
    root: OutcomeEvidenceValidationMember<'session, RecordBlock, SceneGeometryEditEvidenceTag>,
    recipe: &SceneGeometryEditEvidenceRecipe<'_>,
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
                schema::TRIANGULATION_SEMANTIC_VERSION,
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
            counts = row.record_ref::<NodeSceneOutcomeEvidenceSchema, _>(
                schema::field::SCENE_COUNTS,
                |member| member,
            )?;
            corners = row.sequence_ref::<u32, SceneGeometryCornerOriginsTag, _>(
                schema::field::CORNER_ORIGINS,
                |member| member,
            )?;
            faces = row.sequence_ref::<u32, SceneGeometryFaceOffsetsTag, _>(
                schema::field::FACE_OFFSETS,
                |member| member,
            )?;
            Ok(valid)
        })?;
    let (Some(counts), Some(corners), Some(faces)) = (counts, corners, faces) else {
        return Ok(None);
    };
    if !complete {
        return Ok(None);
    }
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
        |index, actual| recipe.source_face_triangle_offsets().get(index).copied() == Some(actual),
    )? {
        return Ok(None);
    }
    Ok(Some(counts))
}

impl SceneGeometryEditFailureEvidence {
    /// Validates the fixed two-member preflight/materialization failure graph.
    pub fn validate_failure<'session, 'storage, 'control, O, PP, U>(
        &self,
        session: &mut FailureEvidenceValidationSession<'session, 'storage, 'control, O, PP, U>,
        root: FailureEvidenceMember<'session, RecordBlock, SceneGeometryEditEvidenceTag>,
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
                    schema::TRIANGULATION_SEMANTIC_VERSION,
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

impl SceneGeometryEditEvidenceRecipe<'_> {
    /// Central rejection keeps the already-rooted changed evidence; visit all four members on
    /// rich failure too, without losing its retained remap owners or claiming a published Scene.
    pub fn validate_failure<'session, 'storage, 'control, O, PP, U>(
        &self,
        session: &mut FailureEvidenceValidationSession<'session, 'storage, 'control, O, PP, U>,
        root: FailureEvidenceMember<'session, RecordBlock, SceneGeometryEditEvidenceTag>,
    ) -> bool
    where
        O: OperationOwner,
        PP: ProgressPhase,
        U: ProgressUnit,
    {
        let (mut counts, mut corners, mut faces) = (None, None, None);
        let valid = session
            .visit_records(root, |index, row| {
                let mut valid = index == 0 && row.fields() == schema::ROOT_FIELDS;
                for (field, expected) in [
                    schema::REVISION,
                    schema::OUTCOME_CHANGED,
                    schema::TRIANGULATION_SEMANTIC_VERSION,
                ]
                .into_iter()
                .chain(self.facts.words)
                .enumerate()
                {
                    valid &= row
                        .scalar::<u32, _>(field, |value| value == expected)
                        .unwrap_or(false);
                }
                for (word, expected) in self.facts.digest.into_iter().enumerate() {
                    valid &= row
                        .scalar::<u64, _>(schema::field::OPTIONS_DIGEST_FIRST + word, |value| {
                            value == expected
                        })
                        .unwrap_or(false);
                }
                counts = row
                    .record_ref::<NodeSceneOutcomeEvidenceSchema, _>(
                        schema::field::SCENE_COUNTS,
                        |member| member,
                    )
                    .ok();
                corners = row
                    .sequence_ref::<u32, SceneGeometryCornerOriginsTag, _>(
                        schema::field::CORNER_ORIGINS,
                        |member| member,
                    )
                    .ok();
                faces = row
                    .sequence_ref::<u32, SceneGeometryFaceOffsetsTag, _>(
                        schema::field::FACE_OFFSETS,
                        |member| member,
                    )
                    .ok();
                valid
            })
            .unwrap_or(false);
        let (Some(counts), Some(corners), Some(faces)) = (counts, corners, faces) else {
            return false;
        };
        valid
            && session
                .visit_sequence(corners, |index, actual| {
                    self.source_corner_triangles()
                        .get(index / 3)
                        .is_some_and(|triangle| triangle[index % 3] == actual)
                })
                .unwrap_or(false)
            && session
                .visit_sequence(faces, |index, actual| {
                    self.source_face_triangle_offsets().get(index).copied() == Some(actual)
                })
                .unwrap_or(false)
            && self.scene_counts().validate_failure(session, counts)
    }
}
