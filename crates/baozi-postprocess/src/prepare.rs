//! Finite selected triangle preparation: lowering, normals, tangents, and one publication.

#[cfg(test)]
mod cancellation_tests;
#[cfg(test)]
pub(crate) mod cleanup_tests;
mod input;
mod options;
mod report;

pub use options::{
    NormalOrientation, PostProcessBudgets, TangentPolicy, TriangleLowering,
    TrianglePreparationOptions,
};
pub use report::{GeometryPreparationReport, PreparedPrimitiveReport};

use crate::direct_error::FailureOwner;
use crate::operation::runtime::{REJECTION_CODE, ScratchVec};
use crate::scene_edit::{
    EditCanonicalSlots, EditCause, EditFailure, EditTerminal, PreparedEdit, validate_changed,
};
use crate::{
    PostProcessControl, PostProcessReport, PostProcessResourceEvidence, PostProcessRunError,
    PostProcessRunFailure, PostProcessRunOutcome,
};
use baozi_core::__scene_integration::integration::{
    SceneEditBootstrap, SceneEditEvidence, SceneGeometryReplacement, SceneTriangleOrigins,
    run_scene_edit_body,
};
use baozi_core::{PrimitiveId, ValidatedScene};
use baozi_operation::__retained_store_integration::activation::CanonicalBoundBodyError;

/// Scalar counts for the selected evidence wire format, accumulated with replacement admission.
#[derive(Default)]
struct SelectionEvidenceWork {
    corner_fields: u32,
    target_and_face_fields: u32,
}

impl SelectionEvidenceWork {
    fn include(&mut self, triangles: usize, face_offsets: usize) -> Option<()> {
        // Each target contributes the fixed header defined by core's selected evidence schema.
        const TARGET_FIELDS: u32 = 18;
        let corners = u32::try_from(triangles).ok()?.checked_mul(3)?;
        let faces = u32::try_from(face_offsets)
            .ok()?
            .checked_add(TARGET_FIELDS)?;
        let corner_fields = self.corner_fields.checked_add(corners)?;
        let target_and_face_fields = self.target_and_face_fields.checked_add(faces)?;
        self.corner_fields = corner_fields;
        self.target_and_face_fields = target_and_face_fields;
        Some(())
    }

    fn validation_work(&self) -> Option<u64> {
        SceneEditBootstrap::selected_evidence_field_work(
            self.corner_fields,
            self.target_and_face_fields,
        )
    }
}

/// Prepare a generation-qualified selection atomically with explicit policies and limits.
/// Source positions and all retained attributes remain borrowed until the one final transaction.
#[allow(clippy::result_large_err)]
pub fn prepare_triangles(
    input: ValidatedScene,
    targets: &[PrimitiveId],
    options: TrianglePreparationOptions,
    control: PostProcessControl,
) -> Result<PostProcessRunOutcome, PostProcessRunFailure> {
    #[cfg(feature = "tracing")]
    let _span = tracing::info_span!("baozi.postprocess.prepare_triangles").entered();
    run(input, targets, options, control, Invocation::Preparation)
}

#[derive(Clone, Copy)]
pub(crate) enum Invocation {
    Preparation,
    Triangulation(PrimitiveId),
    Normals(PrimitiveId, crate::NormalOptions),
    Tangents(PrimitiveId, crate::TangentOptions),
}

/// Typed preparation causes also retain the existing single-operation numeric vocabulary.
#[derive(Debug, thiserror::Error)]
pub enum PreparationCause {
    #[error("the target primitive could not be looked up: {0}")]
    Target(baozi_core::SceneLookupError),
    #[error("the selected primitive appears more than once")]
    DuplicateTarget,
    #[error("the selected primitive has unsupported topology: {0:?}")]
    UnsupportedTopology(baozi_core::TopologyKind),
    #[error("the Scene front-face declaration is unknown; an explicit assumption is required")]
    UnknownFrontFace,
    #[error("preserving a tangent invalidated by normal replacement requires an explicit policy")]
    InvalidatedTangent,
    #[error("triangle preparation failed: {0}")]
    Triangulation(crate::TriangulationCause),
    #[error("normal preparation failed: {0}")]
    Normal(crate::NormalCause),
    #[error("tangent preparation failed: {0}")]
    Tangent(crate::TangentCause),
    #[error("operation control stopped final publication")]
    FinalControl,
    #[error("preparation was rejected by the canonical transaction")]
    Transaction,
    #[error("preparation would invalidate authored {0:?} morph deltas")]
    MorphDependency(baozi_core::MorphDeltaSemantic),
    #[error("preparation would narrow the {0:?} dependency")]
    PrecisionLoss(baozi_core::AttributeSemanticKind),
    #[error("preparation could not start: {0}")]
    Start(baozi_operation::__retained_store_integration::authority::RetainedStoreStartError),
    #[error("preparation was interrupted: {0}")]
    Runtime(baozi_operation::StoreRuntimeError),
    #[error("preparation stopped: {0}")]
    Control(crate::PostProcessControlCause),
    #[error("preparation requires fail-closed containment: {0}")]
    Containment(
        baozi_operation::__retained_store_integration::activation::StoreActivationContainment,
    ),
    #[error("preparation could not freeze its terminal: {0}")]
    Publication(baozi_operation::StorePrepareError),
}

pub struct PreparationFailure {
    cause: PreparationCause,
    resources: Option<PostProcessResourceEvidence>,
    owner: FailureOwner,
}
impl PreparationFailure {
    pub const fn cause(&self) -> &PreparationCause {
        &self.cause
    }
    pub const fn resources(&self) -> Option<&PostProcessResourceEvidence> {
        self.resources.as_ref()
    }
    fn new(cause: PreparationCause) -> Self {
        Self {
            cause,
            resources: None,
            owner: FailureOwner::None,
        }
    }
}
impl std::fmt::Debug for PreparationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparationFailure")
            .field("cause", &self.cause)
            .field("resources", &self.resources)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Display for PreparationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.cause.fmt(f)
    }
}
impl std::error::Error for PreparationFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}
impl From<baozi_operation::StoreRuntimeError> for PreparationCause {
    fn from(e: baozi_operation::StoreRuntimeError) -> Self {
        match e {
            baozi_operation::StoreRuntimeError::Control(c) => {
                Self::Control(crate::error::project_control_cause(c))
            }
            e => Self::Runtime(e),
        }
    }
}
impl From<EditFailure> for PreparationFailure {
    fn from(e: EditFailure) -> Self {
        let cause = match e.cause {
            EditCause::Runtime { source } => source.into(),
            EditCause::Control { cause } => PreparationCause::Control(cause),
            EditCause::FinalControl => PreparationCause::FinalControl,
            EditCause::MorphDependency { semantic } => PreparationCause::MorphDependency(semantic),
            EditCause::PrecisionLoss { semantic } => PreparationCause::PrecisionLoss(semantic),
            EditCause::Containment { source } => PreparationCause::Containment(source),
            EditCause::Preparation { source } => PreparationCause::Publication(source),
            _ => PreparationCause::Transaction,
        };
        Self {
            cause,
            resources: e.resources,
            owner: e.owner,
        }
    }
}

#[allow(clippy::result_large_err)]
pub(crate) fn run(
    input: ValidatedScene,
    targets: &[PrimitiveId],
    options: TrianglePreparationOptions,
    control: PostProcessControl,
    invocation: Invocation,
) -> Result<PostProcessRunOutcome, PostProcessRunFailure> {
    let bootstrap = match SceneEditBootstrap::try_new() {
        Ok(b) => b,
        Err(_) => {
            return Err(PostProcessRunFailure::new(
                input,
                PostProcessRunError::ResourcePolicyInitialization,
            ));
        }
    };
    let (writer_limits, failure_requirements, admission) = bootstrap.into_parts();
    let Some((keys, ledger, start)) =
        crate::operation::runtime::plan_selected_runtime(writer_limits, options.budgets)
    else {
        return Err(PostProcessRunFailure::new(
            input,
            PostProcessRunError::ResourcePolicyInitialization,
        ));
    };
    let mut invocation_host = crate::operation::runtime::EditHost::new();
    let source_generation = input.generation();
    let mut body_cause = None;
    let mut body_containment = None;
    let source = &input;
    let local_cause = &mut body_cause;
    let containment = &mut body_containment;
    let mut activation = Some((admission, failure_requirements));
    let body = run_scene_edit_body(
        &mut invocation_host,
        source,
        control.into_runtime(),
        ledger,
        start,
        move |parts, disposition| {
            let mut startup = parts.startup;
            let prepared = match startup.with_action_execution(keys.work, |runtime| {
                let prepared = input::prepare(source, targets, options, runtime, keys)?;
                #[cfg(test)]
                cleanup_tests::preparation_returned(&prepared);
                Ok::<_, PreparationCause>(prepared)
            }) {
                Ok(prepared) => prepared,
                Err(cause) => {
                    *local_cause = Some(cause);
                    if let Err(cause) = startup.request_failure(disposition, REJECTION_CODE) {
                        *local_cause = Some(cause.into());
                    }
                    return;
                }
            };
            let prepared = match prepared {
                input::PreparedSelection::Unchanged => {
                    if let Err(cause) = startup.request_unchanged(disposition) {
                        *local_cause = Some(cause.into());
                    }
                    return;
                }
                input::PreparedSelection::Prepared(prepared) => prepared,
            };
            let mut operation = match startup.attach() {
                Ok(operation) => operation,
                Err(cause) => {
                    drop(prepared);
                    *local_cause = Some(PreparationCause::Start(cause));
                    return;
                }
            };
            let mut replacements =
                match crate::operation::runtime::with_execution(&mut operation, keys, |runtime| {
                    ScratchVec::new(runtime, keys, prepared.values().len())
                }) {
                    Ok(replacements) => replacements,
                    Err(cause) => {
                        drop(prepared);
                        *local_cause = Some(cause.into());
                        *containment = operation
                            .request_minimal_fallback(disposition, REJECTION_CODE)
                            .err();
                        return;
                    }
                };
            let mut evidence_work = SelectionEvidenceWork::default();
            let append_result =
                crate::operation::runtime::with_execution(&mut operation, keys, |runtime| {
                    for p in prepared.values() {
                        let mut edit = SceneGeometryReplacement::new(p.target);
                        edit.origins = p.origins.as_ref().map(|o| SceneTriangleOrigins {
                            triangles: o.triangles.values(),
                            face_offsets: o.face_offsets.values(),
                        });
                        edit.normals = p.normals.as_ref().map(|n| n.values());
                        edit.tangents = p.tangents.as_ref().map(|t| t.values());
                        edit.remove_tangents = p.remove_tangents;
                        edit.smooth = p.smooth;
                        edit.normal_domain = p.normal_domain;
                        edit.uv_set = p.uv_set;
                        edit.uv_domain = p.uv_domain;
                        let appended = crate::operation::runtime::run_unit(runtime, || {
                            let (triangles, face_offsets) = edit
                                .origins
                                .map_or((0, 0), |o| (o.triangles.len(), o.face_offsets.len()));
                            evidence_work.include(triangles, face_offsets)?;
                            replacements.values_mut().push(edit);
                            Some(())
                        });
                        let cause = match appended {
                            Ok(Some(())) => continue,
                            Ok(None) => baozi_operation::StoreRuntimeError::Plan(
                                baozi_operation::StorePlanError::SizeOverflow,
                            ),
                            Err(e) => e,
                        };
                        return Err(cause);
                    }
                    Ok(())
                });
            if let Err(cause) = append_result {
                drop(replacements);
                drop(prepared);
                *local_cause = Some(cause.into());
                *containment = operation
                    .request_minimal_fallback(disposition, REJECTION_CODE)
                    .err();
                return;
            }
            let qualified = evidence_work
                .validation_work()
                .ok_or(baozi_operation::StoreRuntimeError::Plan(
                    baozi_operation::StorePlanError::SizeOverflow,
                ))
                .and_then(|work| {
                    operation.qualify_failure_validation_work(
                        keys.work,
                        SceneEditBootstrap::success_evidence_members(),
                        work,
                    )
                });
            if let Err(cause) = qualified {
                drop(replacements);
                drop(prepared);
                *local_cause = Some(cause.into());
                *containment = operation
                    .request_minimal_fallback(disposition, REJECTION_CODE)
                    .err();
                return;
            }
            let mappings =
                match baozi_core::__scene_integration::integration::SceneGeometryPlanStorage::try_new(
                    &mut operation,
                    keys.planning,
                    keys.scratch,
                    replacements.values().len(),
                ) {
                    Ok(mappings) => mappings,
                    Err(cause) => {
                        drop(replacements);
                        drop(prepared);
                        *local_cause = Some(cause.into());
                        *containment = operation
                            .request_minimal_fallback(disposition, REJECTION_CODE)
                            .err();
                        return;
                    }
                };
            let (admission, failure_requirements) = activation
                .take()
                .expect("the retained edit body activates at most once");
            let owner = match operation.into_session_outcome_owner(
                keys.session_policy(),
                writer_limits,
                failure_requirements,
            ) {
                Ok(owner) => owner,
                Err(cause) => {
                    drop(mappings);
                    drop(replacements);
                    drop(prepared);
                    *containment = Some(cause);
                    return;
                }
            };
            #[cfg(test)]
            cleanup_tests::selection_ready(&prepared, replacements.values());
            let requested = validate_changed(
                source,
                owner,
                EditCanonicalSlots {
                    output: parts.output,
                    proof: parts.proof,
                    activation: parts.activation,
                    validation: parts.validation,
                },
                admission,
                PreparedEdit::Selection {
                    replacements: replacements.values(),
                    mappings,
                    digest: options.digest(),
                    limits: baozi_core::SceneValidationLimits::default()
                        .with_max_work(options.budgets.max_work_actions)
                        .with_max_scratch_bytes(options.budgets.storage_bytes)
                        .with_max_canonical_bytes(options.budgets.storage_bytes),
                },
                disposition,
            );
            drop(replacements);
            drop(prepared);
            *containment = requested.err();
        },
    );
    let result = match body {
        Ok(finish) => crate::scene_edit::finish_bound_edit(finish),
        Err(CanonicalBoundBodyError::Start(cause)) => {
            return Err(failure(
                input,
                PreparationFailure::new(PreparationCause::Start(cause)),
                invocation,
            ));
        }
        Err(CanonicalBoundBodyError::Body(error)) => {
            if let Some(source) = error.containment().copied().or(body_containment) {
                Err(EditFailure::new(EditCause::Containment { source }))
            } else {
                return Err(failure(
                    input,
                    PreparationFailure::new(body_cause.unwrap_or(PreparationCause::Transaction)),
                    invocation,
                ));
            }
        }
    };
    match result {
        Ok(EditTerminal::Unchanged { resources }) => {
            let report = report_for(
                invocation,
                options,
                source_generation,
                source_generation,
                resources,
                None,
            );
            Ok(PostProcessRunOutcome::new(input, report))
        }
        Ok(EditTerminal::Changed {
            scene,
            evidence: SceneEditEvidence::Selection(evidence),
            resources,
        }) => {
            let report = report_for(
                invocation,
                options,
                source_generation,
                scene.generation(),
                resources,
                Some(evidence),
            );
            Ok(PostProcessRunOutcome::new(scene, report))
        }
        Ok(_) => Err(failure(
            input,
            PreparationFailure::new(PreparationCause::Transaction),
            invocation,
        )),
        Err(error) => {
            let mut error = PreparationFailure::from(error);
            if let Some(cause) = body_cause {
                error.cause = match error.cause {
                    PreparationCause::Transaction => cause,
                    PreparationCause::FinalControl => match cause {
                        cause @ PreparationCause::Control(_) => cause,
                        _ => PreparationCause::FinalControl,
                    },
                    cause => cause,
                };
            }
            Err(failure(input, error, invocation))
        }
    }
}

fn report_for(
    invocation: Invocation,
    options: TrianglePreparationOptions,
    input_generation: baozi_core::SceneGenerationId,
    output_generation: baozi_core::SceneGenerationId,
    resources: PostProcessResourceEvidence,
    evidence: Option<baozi_core::__scene_integration::integration::SceneGeometrySelectionEvidence>,
) -> PostProcessReport {
    match invocation {
        Invocation::Preparation => PostProcessReport::preparation(
            input_generation,
            options,
            output_generation,
            resources,
            evidence.map(GeometryPreparationReport::new),
        ),
        Invocation::Triangulation(target) => PostProcessReport::triangulation(
            target,
            output_generation,
            resources,
            evidence.map(crate::TriangulationRemap::selected),
        ),
        Invocation::Normals(target, options) => match evidence {
            Some(evidence)
                if evidence
                    .target(0)
                    .is_some_and(|t| t.normal_attribute_index().is_some()) =>
            {
                PostProcessReport::normals(
                    target,
                    options,
                    output_generation,
                    resources,
                    Some(crate::NormalGenerationReport::selected(evidence)),
                )
            }
            Some(evidence) => PostProcessReport::normal_preparation(
                target,
                options,
                output_generation,
                resources,
                GeometryPreparationReport::new(evidence),
            ),
            None => PostProcessReport::normals(target, options, output_generation, resources, None),
        },
        Invocation::Tangents(target, options) => PostProcessReport::tangents(
            target,
            options,
            output_generation,
            resources,
            evidence.map(crate::TangentGenerationReport::selected),
        ),
    }
}

fn failure(
    input: ValidatedScene,
    e: PreparationFailure,
    invocation: Invocation,
) -> PostProcessRunFailure {
    use crate::{NormalCause as N, TangentCause as T, TriangulationCause as P};
    let PreparationFailure {
        cause,
        resources,
        owner,
    } = e;
    let error = match invocation {
        Invocation::Preparation => PostProcessRunError::Preparation {
            failure: PreparationFailure {
                cause,
                resources,
                owner,
            },
        },
        Invocation::Normals(..) => {
            let c = match cause {
                PreparationCause::Normal(c) => c,
                PreparationCause::Tangent(crate::TangentCause::MultipleTangentFields) => {
                    N::MultipleTangentFields
                }
                PreparationCause::Tangent(cause) => N::Tangent { cause },
                PreparationCause::FinalControl => N::FinalControl,
                PreparationCause::Target(source) => N::Target { source },
                PreparationCause::UnsupportedTopology(_) => N::UnsupportedTopology,
                PreparationCause::InvalidatedTangent => N::TangentsRequireRemoval,
                PreparationCause::MorphDependency(semantic) => N::MorphDeltas { semantic },
                PreparationCause::PrecisionLoss(semantic) => N::PrecisionLoss { semantic },
                PreparationCause::Runtime(source) => source.into(),
                PreparationCause::Control(cause) => N::Control { cause },
                PreparationCause::Start(source) => N::Start { source },
                PreparationCause::Containment(source) => N::Containment { source },
                PreparationCause::Publication(source) => N::Preparation { source },
                _ => N::CoreRejected,
            };
            PostProcessRunError::Normals {
                failure: match resources {
                    Some(r) => crate::NormalFailure::terminal(c, r, owner),
                    None => crate::NormalFailure::new(c),
                },
            }
        }
        Invocation::Tangents(..) => {
            let c = match cause {
                PreparationCause::Tangent(c) => c,
                PreparationCause::FinalControl => T::FinalControl,
                PreparationCause::Target(source) => T::Target { source },
                PreparationCause::UnsupportedTopology(_) => T::UnsupportedTopology,
                PreparationCause::MorphDependency(_) => T::MorphDeltas,
                PreparationCause::PrecisionLoss(semantic) => T::PrecisionLoss { semantic },
                PreparationCause::Runtime(source) => source.into(),
                PreparationCause::Control(cause) => T::Control { cause },
                PreparationCause::Start(source) => T::Start { source },
                PreparationCause::Containment(source) => T::Containment { source },
                PreparationCause::Publication(source) => T::Preparation { source },
                _ => T::CoreRejected,
            };
            PostProcessRunError::Tangents {
                failure: match resources {
                    Some(r) => crate::TangentFailure::terminal(c, r, owner),
                    None => crate::TangentFailure::new(c),
                },
            }
        }
        Invocation::Triangulation(_) => {
            let c = match cause {
                PreparationCause::Triangulation(c) => c,
                PreparationCause::FinalControl => P::FinalControl,
                PreparationCause::Target(source) => P::Target { source },
                PreparationCause::UnsupportedTopology(topology) => {
                    P::UnsupportedTopology { topology }
                }
                PreparationCause::Runtime(source) => source.into(),
                PreparationCause::Control(cause) => P::Control { cause },
                PreparationCause::Start(source) => P::Start { source },
                PreparationCause::Containment(source) => P::Containment { source },
                PreparationCause::Publication(source) => P::Preparation { source },
                _ => P::CoreRejected,
            };
            PostProcessRunError::Triangulation {
                failure: match resources {
                    Some(r) => crate::TriangulationFailure::terminal(c, r, owner),
                    None => crate::TriangulationFailure::new(c),
                },
            }
        }
    };
    PostProcessRunFailure::new(input, error)
}

#[cfg(test)]
mod terminal_work_tests {
    use super::*;

    #[test]
    fn selection_counts_include_fixed_records_each_target_header_and_actual_origins() {
        let mut work = SelectionEvidenceWork::default();
        work.include(2, 2).unwrap();
        work.include(0, 0).unwrap();
        work.include(3, 4).unwrap();
        assert_eq!(work.corner_fields, 15);
        assert_eq!(work.target_and_face_fields, 18 * 3 + 2 + 4);
        assert_eq!(
            work.validation_work(),
            Some(SceneEditBootstrap::failure_field_work() + 76)
        );
    }

    #[test]
    fn selection_count_overflow_is_finite_and_preserves_the_prior_prefix() {
        let mut work = SelectionEvidenceWork::default();
        work.include(1, 2).unwrap();
        let before = work.validation_work();
        for (triangles, faces) in [
            (usize::MAX, 0),
            (0, usize::MAX),
            ((u32::MAX / 3) as usize + 1, 0),
        ] {
            assert_eq!(work.include(triangles, faces), None);
            assert_eq!(work.validation_work(), before);
        }
        let mut corners = SelectionEvidenceWork::default();
        corners.include((u32::MAX / 3) as usize, 0).unwrap();
        assert_eq!(corners.corner_fields, u32::MAX);
        let before = corners.validation_work();
        assert_eq!(corners.include(1, 0), None);
        assert_eq!(corners.validation_work(), before);
        let mut faces = SelectionEvidenceWork::default();
        faces.include(0, (u32::MAX - 18) as usize).unwrap();
        assert_eq!(faces.target_and_face_fields, u32::MAX);
        let before = faces.validation_work();
        assert_eq!(faces.include(0, 0), None);
        assert_eq!(faces.validation_work(), before);
    }

    #[test]
    fn default_work_and_storage_budgets_admit_empty_selection() {
        let scene = crate::test_fixtures::canonical_triangle_scene();
        let generation = scene.generation();
        let result = prepare_triangles(
            scene,
            &[],
            TrianglePreparationOptions::default(),
            PostProcessControl::default(),
        )
        .unwrap();
        assert_eq!(result.scene().generation(), generation);
        assert_eq!(
            result.report().disposition(),
            crate::PostProcessRunDisposition::Unchanged
        );
        assert_eq!(result.report().resources().current_live_bytes(), 0);
        assert_eq!(result.report().resources().active_reservations(), 0);
    }

    #[test]
    fn default_work_and_storage_budgets_publish_the_actual_quad_evidence() {
        let scene = crate::test_fixtures::canonical_quad_scene();
        let target = scene
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap()
            .id();
        let generation = scene.generation();
        let result = prepare_triangles(
            scene,
            &[target],
            TrianglePreparationOptions::default(),
            PostProcessControl::default(),
        )
        .unwrap();
        assert_ne!(result.scene().generation(), generation);
        assert_eq!(
            result.report().disposition(),
            crate::PostProcessRunDisposition::Changed
        );
        assert_eq!(result.report().geometry_preparation().unwrap().len(), 1);
        let resources = result.report().resources();
        assert_eq!(resources.current_live_bytes(), 0);
        assert_eq!(resources.active_reservations(), 0);
        assert!(
            resources.attempted_work_actions() <= PostProcessBudgets::default().max_work_actions
        );
        assert!(resources.consumed_work_actions() <= resources.attempted_work_actions());
        assert!(resources.published_outcome_evidence_bytes() > 0);
    }

    #[test]
    fn small_admitted_work_budget_returns_the_input_with_terminal_evidence() {
        let (scene, import) = crate::test_fixtures::canonical_quad_import().into_parts();
        let target = scene
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap()
            .id();
        let generation = scene.generation();
        let reserve =
            baozi_operation::RetainedStoreFailureTerminalWorkPlan::requirement_for_evidence_work(
                SceneEditBootstrap::failure_evidence_members(),
                SceneEditBootstrap::failure_field_work(),
                SceneEditBootstrap::failure_field_work(),
            )
            .unwrap()
            .attempted_work_limit();
        let options = TrianglePreparationOptions {
            budgets: PostProcessBudgets {
                max_work_actions: reserve + 10,
                ..Default::default()
            },
            ..Default::default()
        };
        let failure = prepare_triangles(scene, &[target], options, PostProcessControl::default())
            .unwrap_err();
        assert_eq!(failure.input().generation(), generation);
        assert!(import.matches_scene(failure.input()));
        let PostProcessRunError::Preparation { failure } = failure.error() else {
            panic!("selected preparation failure")
        };
        let resources = failure
            .resources()
            .expect("startup admitted the minimal terminal guarantee");
        assert!(resources.attempted_work_actions() > 0);
        assert!(resources.consumed_work_actions() > 0);
        assert_eq!(resources.current_live_bytes(), 0);
        assert_eq!(resources.active_reservations(), 0);
    }
}
