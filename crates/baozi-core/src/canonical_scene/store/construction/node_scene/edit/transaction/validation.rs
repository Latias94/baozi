//! Central validation proves the actual candidate and every rooted remap scalar together.

use baozi_operation::__retained_store_integration::activation::{
    CanonicalValidationDecision, CanonicalValidationMember, CanonicalValidationPlanning,
    CanonicalValidationSession, FailureEvidenceMember, FailureEvidenceValidationSession,
    OutcomeEvidenceValidationMember,
};
use baozi_operation::{
    OperationOwner, OutcomeEvidenceSequenceRef, OutcomeEvidenceStoreRef, ProgressPhase,
    ProgressUnit, RecordBlock, StoreWorkError,
};

#[cfg(test)]
use crate::attribute::AttributeDomain;
use crate::canonical_scene::store::construction::node_scene::scratch::ScopedNodeSceneValidationScratchOwner;
use crate::canonical_scene::store::schema::{NodeSceneOutcomeEvidenceSchema, SceneDirectorySchema};
use crate::canonical_scene::store::validation::{
    NodeSceneValidationProof, validate_node_scene_canonical_graph_metered,
};
#[cfg(test)]
use crate::identity::PrimitiveId;
use crate::identity::SceneGenerationId;
use crate::scene_space::{Axis, FrontFaceWinding, Handedness, SceneSpace};
use crate::space_conversion::{SpaceConversion, TargetSpace};

use super::evidence::{SceneEditEvidenceRecipe, SceneEditFacts};
use super::{
    SceneEditEvidenceTag, SceneEditFailureEvidence, SceneEditOutput, SceneEditPostflight,
    SceneEditRejection, SceneGeometryCornerOriginsTag, SceneGeometryFaceOffsetsTag, schema,
};

/// Proof of the actual canonical graph, scalar correspondence, and rooted edit evidence.
#[derive(Clone, Copy)]
pub struct SceneEditValidationProof {
    pub(super) scene: NodeSceneValidationProof,
    pub(super) evidence_root: OutcomeEvidenceStoreRef<RecordBlock, SceneEditEvidenceTag>,
    pub(super) handoff: SceneEditHandoffFacts,
}

#[derive(Clone, Copy)]
pub(super) struct SceneEditOrigins {
    pub(super) corners: OutcomeEvidenceSequenceRef<u32, SceneGeometryCornerOriginsTag>,
    pub(super) faces: OutcomeEvidenceSequenceRef<u32, SceneGeometryFaceOffsetsTag>,
}

#[derive(Clone, Copy)]
pub(super) enum SceneEditHandoffFacts {
    Selection {
        input_generation: SceneGenerationId,
        targets: u32,
        reconstruction_work_items: u64,
        geometry_reconstruction_count: u32,
        reconstructed_primitive_count: u32,
        removed_metadata: u32,
        origins: SceneEditOrigins,
    },
    SpaceConversion {
        input_generation: SceneGenerationId,
        source: TargetSpace,
        target: TargetSpace,
        assumed_mask: u32,
        reverse_winding: bool,
    },
    #[cfg(test)]
    Triangulation {
        input_target: PrimitiveId,
        origins: SceneEditOrigins,
        source_corners: u32,
        vertices: u32,
    },
    #[cfg(test)]
    Normals {
        input_target: PrimitiveId,
        effects: u32,
        removed_metadata: u32,
        normal_attribute: u32,
    },
    #[cfg(test)]
    Tangents {
        input_target: PrimitiveId,
        replaced: bool,
        tangent_attribute: u32,
        corners: u32,
        normal_domain: AttributeDomain,
        uv_set: u32,
        uv_domain: AttributeDomain,
    },
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
        let scratch_plan = output.scene.inner().central_validation.scratch_plan();
        planning.validate_with(
            |checkpoint| {
                ScopedNodeSceneValidationScratchOwner::try_allocate(checkpoint, scratch_plan)
            },
            |session, directory, evidence, scratch| {
                Ok(
                    match self.prove_candidate(session, directory, evidence, scratch, output)? {
                        Some(proof) => CanonicalValidationDecision::accept(proof),
                        None => CanonicalValidationDecision::reject(
                            SceneEditRejection::construction_invariant(),
                            super::super::super::NODE_SCENE_BUILD_INVARIANT_CODE,
                        ),
                    },
                )
            },
        )
    }

    pub(super) fn prove_candidate<'session, O, PP, U>(
        &self,
        session: &mut CanonicalValidationSession<'session, '_, '_, '_, O, PP, U>,
        directory: CanonicalValidationMember<'session, RecordBlock, SceneDirectorySchema>,
        evidence: OutcomeEvidenceValidationMember<'session, RecordBlock, SceneEditEvidenceTag>,
        scratch: &mut ScopedNodeSceneValidationScratchOwner<'_, O>,
        output: &SceneEditOutput,
    ) -> Result<Option<SceneEditValidationProof>, StoreWorkError>
    where
        O: OperationOwner,
        PP: ProgressPhase,
        U: ProgressUnit,
    {
        let scene = output.scene.inner();
        let plan = scene.central_validation;
        if !scratch.initialize(session, plan.scratch_plan())?
            || scene.brand != self.brand
            || output.source_generation != self.input.generation()
        {
            return Ok(None);
        }
        let Some(storage) = self.storage else {
            return Ok(None);
        };
        let expected = storage.facts(
            self.input.as_node_scene(),
            self.replacement,
            scene.evidence_counts,
        );
        if Some(output.facts) != expected {
            return Ok(None);
        }
        let recipe = self.candidate_evidence(output);
        let Some((counts, handoff)) = validate_changed_root(
            session,
            evidence,
            &recipe,
            self.storage,
            output.source_generation,
        )?
        else {
            return Ok(None);
        };
        if !storage.validate_correspondence(&scene.tables, session)? {
            return Ok(None);
        }
        let mut scratch = scratch.metered_view();
        Ok(validate_node_scene_canonical_graph_metered(
            session,
            directory,
            counts,
            scene.evidence_counts,
            scene.refs,
            &scene.tables,
            plan,
            &mut scratch,
        )?
        .map(|scene| SceneEditValidationProof {
            scene,
            evidence_root: evidence.reference(),
            handoff,
        }))
    }
}

#[allow(clippy::type_complexity)]
fn validate_changed_root<'session, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, '_, '_, '_, O, PP, U>,
    root: OutcomeEvidenceValidationMember<'session, RecordBlock, SceneEditEvidenceTag>,
    recipe: &SceneEditEvidenceRecipe<'_>,
    storage: Option<super::EditStoragePlan<'_>>,
    source_generation: SceneGenerationId,
) -> Result<
    Option<(
        OutcomeEvidenceValidationMember<'session, RecordBlock, NodeSceneOutcomeEvidenceSchema>,
        SceneEditHandoffFacts,
    )>,
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
                (schema::field::REVISION, schema::REVISION),
                (schema::field::OUTCOME, schema::OUTCOME_CHANGED),
                (
                    schema::field::SEMANTIC_VERSION,
                    schema::OPERATION_SEMANTIC_VERSION,
                ),
            ]
            .into_iter()
            .chain(
                recipe
                    .facts
                    .words
                    .into_iter()
                    .enumerate()
                    .map(|(offset, expected)| (schema::field::OPERATION_KIND + offset, expected)),
            ) {
                valid &= row.scalar(field, |value: u32| value)? == Some(expected);
            }
            for (word, expected) in recipe.facts.digest.into_iter().enumerate() {
                valid &= row.scalar(schema::field::OPTIONS_DIGEST_FIRST + word, |value: u64| {
                    value
                })? == Some(expected);
            }
            for (field, expected) in [
                schema::field::SOURCE_UNIT_BITS,
                schema::field::TARGET_UNIT_BITS,
            ]
            .into_iter()
            .zip(recipe.facts.units)
            {
                valid &= row.scalar(field, |value: u64| value)? == Some(expected);
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
        return Ok(if complete {
            counts.and_then(|counts| {
                SceneEditHandoffFacts::from_validated_root(
                    recipe.facts,
                    recipe.replacement.target(),
                    source_generation,
                    None,
                )
                .map(|handoff| (counts, handoff))
            })
        } else {
            None
        });
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
    Ok(SceneEditHandoffFacts::from_validated_root(
        recipe.facts,
        recipe.replacement.target(),
        source_generation,
        Some(SceneEditOrigins {
            corners: corners.reference(),
            faces: faces.reference(),
        }),
    )
    .map(|handoff| (counts, handoff)))
}

impl SceneEditHandoffFacts {
    fn from_validated_root(
        facts: SceneEditFacts,
        input_target: super::SceneEditTarget,
        source_generation: SceneGenerationId,
        origins: Option<SceneEditOrigins>,
    ) -> Option<Self> {
        use super::SceneEditTarget;

        let target_generation = match input_target {
            SceneEditTarget::Primitive(target) => target.generation(),
            SceneEditTarget::WholeScene(generation) => generation,
        };
        if target_generation != source_generation {
            return None;
        }
        let word = |field| facts.words[field - schema::field::OPERATION_KIND];
        match (facts.words[0], input_target) {
            (schema::OPERATION_SELECTION, SceneEditTarget::WholeScene(input_generation)) => {
                Some(Self::Selection {
                    input_generation,
                    targets: facts.words[1],
                    reconstruction_work_items: facts.units[0],
                    geometry_reconstruction_count: facts.words[2],
                    reconstructed_primitive_count: facts.words[3],
                    removed_metadata: facts.words[4],
                    origins: origins?,
                })
            }
            (schema::OPERATION_SPACE_CONVERSION, SceneEditTarget::WholeScene(input_generation)) => {
                if origins.is_some() || word(schema::field::OPTIONS_ALGORITHM) != 0 {
                    return None;
                }
                let source = decode_space(
                    facts,
                    facts.units[0],
                    [
                        schema::space_field::SOURCE_HANDEDNESS,
                        schema::space_field::SOURCE_UP,
                        schema::space_field::SOURCE_FRONT,
                        schema::space_field::SOURCE_WINDING,
                    ],
                )?;
                let target = decode_space(
                    facts,
                    facts.units[1],
                    [
                        schema::space_field::TARGET_HANDEDNESS,
                        schema::space_field::TARGET_UP,
                        schema::space_field::TARGET_FRONT,
                        schema::space_field::TARGET_WINDING,
                    ],
                )?;
                let assumed_mask = word(schema::space_field::ASSUMED_MASK);
                let reverse = word(schema::space_field::REVERSE_WINDING);
                let changes = word(schema::space_field::CHANGES_COORDINATES);
                if assumed_mask > 31 || reverse > 1 || changes > 1 {
                    return None;
                }
                let source_space = SceneSpace {
                    handedness: source.handedness(),
                    up_axis: source.up_axis(),
                    front_axis: source.front_axis(),
                    unit_scale_to_meters: Some(source.unit_scale_to_meters()),
                    front_face_winding: source.front_face_winding(),
                    ..SceneSpace::UNKNOWN
                };
                let conversion = SpaceConversion::try_new(&source_space, target, None).ok()?;
                if conversion.reverse_winding() != (reverse != 0)
                    || conversion.changes_coordinates() != (changes != 0)
                {
                    return None;
                }
                Some(Self::SpaceConversion {
                    input_generation,
                    source,
                    target,
                    assumed_mask,
                    reverse_winding: reverse != 0,
                })
            }
            #[cfg(test)]
            (schema::OPERATION_TRIANGULATE, SceneEditTarget::Primitive(input_target)) => {
                if facts.units != [0; 2] {
                    return None;
                }
                Some(Self::Triangulation {
                    input_target,
                    origins: origins?,
                    source_corners: word(schema::field::SOURCE_CORNER_COUNT),
                    vertices: word(schema::field::VERTEX_COUNT),
                })
            }
            #[cfg(test)]
            (schema::OPERATION_NORMALS, SceneEditTarget::Primitive(input_target)) => {
                let normal_attribute = word(schema::normal_field::OUTPUT_NORMAL_ATTRIBUTE);
                if facts.units != [0; 2]
                    || origins.is_some()
                    || normal_attribute >= word(schema::normal_field::OUTPUT_ATTRIBUTE_COUNT)
                {
                    return None;
                }
                Some(Self::Normals {
                    input_target,
                    effects: word(schema::normal_field::EFFECTS),
                    removed_metadata: word(schema::normal_field::REMOVED_METADATA),
                    normal_attribute,
                })
            }
            #[cfg(test)]
            (schema::OPERATION_TANGENTS, SceneEditTarget::Primitive(input_target)) => {
                let replaced = word(schema::tangent_field::REPLACED_TANGENT);
                let tangent_attribute = word(schema::tangent_field::OUTPUT_TANGENT_ATTRIBUTE);
                let corners = word(schema::tangent_field::CORNER_COUNT);
                if facts.units != [0; 2]
                    || origins.is_some()
                    || replaced > 1
                    || tangent_attribute >= word(schema::tangent_field::OUTPUT_ATTRIBUTE_COUNT)
                    || corners == 0
                    || !corners.is_multiple_of(3)
                    || word(schema::tangent_field::FACE_COUNT) != corners / 3
                {
                    return None;
                }
                Some(Self::Tangents {
                    input_target,
                    replaced: replaced != 0,
                    tangent_attribute,
                    corners,
                    normal_domain: decode_tangent_domain(word(
                        schema::tangent_field::NORMAL_DOMAIN,
                    ))?,
                    uv_set: word(schema::tangent_field::UV_SET),
                    uv_domain: decode_tangent_domain(word(schema::tangent_field::UV_DOMAIN))?,
                })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
fn decode_tangent_domain(value: u32) -> Option<AttributeDomain> {
    match value {
        0 => Some(AttributeDomain::Vertex),
        1 => Some(AttributeDomain::Corner),
        _ => None,
    }
}

fn decode_space(facts: SceneEditFacts, unit: u64, fields: [usize; 4]) -> Option<TargetSpace> {
    let word = |field| facts.words[field - schema::field::OPERATION_KIND];
    let handedness = match word(fields[0]) {
        1 => Handedness::Right,
        2 => Handedness::Left,
        _ => return None,
    };
    let axis = |value| match value {
        1 => Some(Axis::PositiveX),
        2 => Some(Axis::NegativeX),
        3 => Some(Axis::PositiveY),
        4 => Some(Axis::NegativeY),
        5 => Some(Axis::PositiveZ),
        6 => Some(Axis::NegativeZ),
        _ => None,
    };
    let up = axis(word(fields[1]))?;
    let front = axis(word(fields[2]))?;
    let winding = match word(fields[3]) {
        1 => FrontFaceWinding::Clockwise,
        2 => FrontFaceWinding::CounterClockwise,
        _ => return None,
    };
    TargetSpace::try_new(handedness, up, front, f64::from_bits(unit), winding).ok()
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
        CanonicalActivationOutcome, CanonicalActivationSeal, CanonicalFreezePermitIssue,
        CanonicalInvocationOutcome, CanonicalValidationProgress, FailureEvidenceOutcome,
        RootedFailureValidation,
    };
    use baozi_operation::__retained_store_integration::authority::PreparedStoreFailureOutcome;
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
        SceneEditBootstrap, SceneEditPublication, consume_scene_edit_freeze_permit,
        resolve_scene_space_conversion,
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
            let mut invocation_root_1 =
                crate::canonical_scene::store::construction::SceneCanonicalInvocationHost::<
                    super::super::SceneEditOutput,
                    SceneEditValidationProof,
                    super::super::SceneEditRejection,
                    super::super::SceneEditRejection,
                    super::super::SceneEditGenerationRejection,
                    SceneEditEvidenceTag,
                >::new();
            {
                let mut inputs = Some((start, writer, failure, admission));
                let finish = invocation_root_1
                    .run_body(|parts, disposition| {
                        let (start, writer, failure, admission) = inputs.take().unwrap();
                        let operation = SceneStoreOperation::try_start_derived(
                            parts.startup,
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
                        let mut transaction = admission
                            .into_space_transaction(resolved, SceneValidationLimits::default());
                        let activated = owner
                            .plan_and_activate_scoped(
                                parts.output,
                                parts.activation,
                                |mut planning| {
                                    let (sealed, completion) = planning
                                        .with_semantic_work(|semantic| {
                                            transaction.preflight(semantic)
                                        })
                                        .unwrap();
                                    match planning.seal_activation(sealed, completion) {
                                        CanonicalActivationSeal::Sealed(permit) => {
                                            permit.activate(|lane, plan| plan.materialize(lane))
                                        }
                                        CanonicalActivationSeal::Unsealed(failure) => {
                                            failure.finish()
                                        }
                                    }
                                },
                            )
                            .unwrap();
                        let postflight = transaction.finish();
                        let CanonicalActivationOutcome::Activated(activated) = activated else {
                            panic!("revision {revision}: valid space edit must materialize")
                        };
                        let central = activated
                            .validate_centrally(
                                parts.proof,
                                parts.validation,
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
                                            |writer| {
                                                writer.write_row(|row| recipe.counts.write_row(row))
                                            },
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
                                                    row.write_scalar(
                                                        schema::OPERATION_SEMANTIC_VERSION,
                                                    )?;
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
                        central.request_finish(disposition).unwrap();
                    })
                    .unwrap();
                let completion = finish.resume();
                match completion.outcome {
                    CanonicalInvocationOutcome::Candidate(candidate) => {
                        assert_eq!(revision, 1);
                        let CanonicalFreezePermitIssue::Ready(permit) = candidate
                            .try_issue_freeze_permit(
                                completion.generation.interruption,
                                final_progress(),
                            )
                            .unwrap()
                        else {
                            panic!("v1 changed root must reach publication")
                        };
                        let SceneEditPublication::Success(published) =
                            consume_scene_edit_freeze_permit(
                                permit,
                                completion.generation.rejection,
                            )
                            .unwrap()
                        else {
                            panic!("v1 changed root must publish successfully")
                        };
                        let published = published.into_parts();
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
                    CanonicalInvocationOutcome::CentralRejected(rejected) => {
                        assert_ne!(revision, 1);
                        let (prepared, _) =
                            rejected.prepare_minimal_fallback().unwrap().into_parts();
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
            }
            drop(invocation_root_1);
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
            let mut invocation_root_2 =
                crate::canonical_scene::store::construction::SceneCanonicalInvocationHost::<
                    (),
                    (),
                    (),
                    (),
                    (),
                    SceneEditEvidenceTag,
                >::new();
            {
                let mut inputs = Some((start, writer, failure));
                let finish = invocation_root_2
                    .run_body(|parts, disposition| {
                        let (start, writer, failure) = inputs.take().unwrap();
                        let operation = SceneStoreOperation::try_start_derived(
                            parts.startup,
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
                                parts.activation,
                                (),
                                0x5531_3201,
                            )
                            .unwrap();
                        let CanonicalActivationOutcome::NeverActivated(rejected) = rejected else {
                            panic!("fixed preflight rejection must not activate canonical storage")
                        };
                        rejected.request_finish(disposition).unwrap();
                    })
                    .unwrap();
                let completion = finish.resume();
                let CanonicalInvocationOutcome::NeverActivated(rejected) = completion.outcome
                else {
                    panic!("fixed preflight rejection must restore its actual rejected state")
                };
                let rooted = rejected
                    .build_failure_evidence(|lane| {
                        let counts = lane
                            .try_retain_records(
                                evidence.scene_counts().record_plan(),
                                NodeSceneOutcomeEvidenceSchema,
                                |writer| {
                                    writer.write_row(|row| evidence.scene_counts().write_row(row))
                                },
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
                    .validate_rich_failure(completion.failure_fallback, |session, root| {
                        evidence.validate_failure(session, root)
                    })
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
                        let (prepared, _) =
                            rejected.prepare_minimal_fallback().unwrap().into_parts();
                        drop(prepared.handoff());
                    }
                    RootedFailureValidation::Fallback(_) => {
                        panic!("revision {revision}: expected a semantic revision decision")
                    }
                }
            }
            drop(invocation_root_2);
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
