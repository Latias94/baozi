use crate::error::{
    PostProcessCompileFailure, PostProcessError, PostProcessExecutionFailure, PostProcessFailure,
    PostProcessReport, PostProcessRunError, PostProcessRunOutcome, project_control_cause,
};
use crate::operation::{
    PostProcessControl, PostProcessDimension, PostProcessLedgerPhase, PostProcessOwner,
    PostProcessProgressPhase, PostProcessProgressUnit, RuntimePostProcessControl,
    default_ledger_limits,
};
use baozi_core::__canonical_geometry::{SceneGenerationId, ValidatedScene};
use baozi_operation::{LedgerCause, LedgerLimits, LedgerLimitsError, OperationCoordinator};

type Coordinator = OperationCoordinator<
    PostProcessOwner,
    PostProcessLedgerPhase,
    PostProcessProgressPhase,
    PostProcessProgressUnit,
    PostProcessDimension,
>;

fn try_postprocess_coordinator(
    control: RuntimePostProcessControl,
    ledger_limits: LedgerLimits<PostProcessDimension>,
) -> std::result::Result<Coordinator, LedgerCause<PostProcessDimension>> {
    Coordinator::try_new(control, ledger_limits, &PostProcessLedgerPhase::ALL)
}

/// An explicit post-process request set.
///
/// U7 currently exposes only the empty request set. Canonical transformations
/// become constructible only when their descriptor, controlled compiler,
/// typed edit program, report schema, and tests land together.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PostProcessRequests {
    _private: (),
}

impl PostProcessRequests {
    /// Creates an explicit request set containing no transformations.
    pub const fn empty() -> Self {
        Self { _private: () }
    }

    /// Compiles against an owned validated Scene with default control.
    #[allow(
        clippy::result_large_err,
        reason = "compile failure returns the original linear Scene and complete bounded evidence"
    )]
    pub fn compile(
        self,
        input: ValidatedScene,
    ) -> std::result::Result<CompiledPostProcessRun, PostProcessCompileFailure> {
        self.compile_with_control(input, PostProcessControl::default())
    }

    /// Compiles with invocation-local cancellation and deadline control.
    #[allow(
        clippy::result_large_err,
        reason = "compile failure returns the original linear Scene and complete bounded evidence"
    )]
    pub fn compile_with_control(
        self,
        input: ValidatedScene,
        control: PostProcessControl,
    ) -> std::result::Result<CompiledPostProcessRun, PostProcessCompileFailure> {
        self.compile_with_control_and_ledger_limits(input, control, default_ledger_limits())
    }

    #[allow(
        clippy::result_large_err,
        reason = "compile failure returns the original linear Scene and complete bounded evidence"
    )]
    fn compile_with_control_and_ledger_limits(
        self,
        input: ValidatedScene,
        control: PostProcessControl,
        ledger_limits: std::result::Result<
            LedgerLimits<PostProcessDimension>,
            LedgerLimitsError<PostProcessDimension>,
        >,
    ) -> std::result::Result<CompiledPostProcessRun, PostProcessCompileFailure> {
        let ledger_limits = match ledger_limits {
            Ok(limits) => limits,
            Err(_) => {
                return Err(PostProcessCompileFailure::new(
                    input,
                    PostProcessRunError::resource_policy_initialization(),
                ));
            }
        };
        let mut coordinator =
            match try_postprocess_coordinator(control.into_runtime(), ledger_limits) {
                Ok(coordinator) => coordinator,
                Err(_) => {
                    return Err(PostProcessCompileFailure::new(
                        input,
                        PostProcessRunError::coordinator_initialization(),
                    ));
                }
            };
        #[cfg(feature = "tracing")]
        let _span = tracing::info_span!("baozi.postprocess.compile").entered();

        if let Err(cause) = coordinator.poll() {
            return Err(PostProcessCompileFailure::new(
                input,
                PostProcessRunError::operation(finish_failure(
                    coordinator,
                    PostProcessError::Control {
                        cause: project_control_cause(cause),
                    },
                )),
            ));
        }

        let plan = CompiledPostProcessPlan::empty(input.generation());
        Ok(CompiledPostProcessRun {
            input,
            plan,
            coordinator,
        })
    }
}

/// The immutable plan compiled against the Scene owned by its run.
#[derive(Debug, PartialEq, Eq)]
pub struct CompiledPostProcessPlan {
    input_generation: SceneGenerationId,
    steps: Vec<CompiledPostProcessStep>,
    requires_canonical_write: bool,
}

impl CompiledPostProcessPlan {
    fn empty(input_generation: SceneGenerationId) -> Self {
        Self {
            input_generation,
            steps: Vec::new(),
            requires_canonical_write: false,
        }
    }

    /// Returns the generation against which this plan was compiled.
    pub const fn input_generation(&self) -> SceneGenerationId {
        self.input_generation
    }

    /// Returns the deterministic source-ordered compiled steps.
    pub fn steps(&self) -> &[CompiledPostProcessStep] {
        &self.steps
    }

    /// Returns whether the normalized request set contains no steps.
    pub const fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Returns the number of explicit compiled steps.
    pub const fn step_count(&self) -> usize {
        self.steps.len()
    }

    /// Returns whether execution must open a canonical edit transaction.
    pub const fn requires_canonical_write(&self) -> bool {
        self.requires_canonical_write
    }
}

/// One normalized operation in a compiled post-process plan.
///
/// The empty-only U7 slice produces no values. Future operations extend this
/// opaque type together with their descriptor, edit program, report, and tests.
#[derive(Debug, PartialEq, Eq)]
pub struct CompiledPostProcessStep {
    _private: (),
}

/// A linear post-process run bound to one owned validated Scene and one ledger.
#[must_use = "a compiled post-process run must be executed or intentionally dropped"]
pub struct CompiledPostProcessRun {
    input: ValidatedScene,
    plan: CompiledPostProcessPlan,
    coordinator: Coordinator,
}

impl std::fmt::Debug for CompiledPostProcessRun {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CompiledPostProcessRun")
            .field("input_generation", &self.input.generation())
            .field(
                "input_canonical_block_count",
                &self.input.canonical_block_count(),
            )
            .field("plan", &self.plan)
            .finish_non_exhaustive()
    }
}

impl CompiledPostProcessRun {
    /// Returns the immutable plan bound to the owned input Scene.
    pub const fn plan(&self) -> &CompiledPostProcessPlan {
        &self.plan
    }

    /// Executes the compiled request set.
    ///
    /// The empty plan performs one final control poll, opens no edit, allocates
    /// no canonical output, and returns the original Scene owner unchanged.
    #[allow(
        clippy::result_large_err,
        reason = "execution failure returns the original linear Scene and complete bounded evidence"
    )]
    pub fn execute(
        self,
    ) -> std::result::Result<PostProcessRunOutcome, PostProcessExecutionFailure> {
        let Self {
            input,
            plan,
            mut coordinator,
        } = self;
        #[cfg(feature = "tracing")]
        let _span = tracing::info_span!("baozi.postprocess.execute").entered();

        if let Err(cause) = coordinator.poll() {
            return Err(PostProcessExecutionFailure::new(
                input,
                finish_failure(
                    coordinator,
                    PostProcessError::Control {
                        cause: project_control_cause(cause),
                    },
                ),
            ));
        }

        match coordinator.finish() {
            Ok(ledger) => Ok(PostProcessRunOutcome::new(
                input,
                PostProcessReport::unchanged(plan, ledger),
            )),
            Err(finalization_error) => Err(PostProcessExecutionFailure::new(
                input,
                PostProcessFailure::from_finalization_failure(finalization_error),
            )),
        }
    }
}

fn finish_failure(coordinator: Coordinator, error: PostProcessError) -> PostProcessFailure {
    match coordinator.finish() {
        Ok(ledger) => PostProcessFailure::new(error, ledger),
        Err(finalization_error) => {
            PostProcessFailure::with_cleanup_failure(error, finalization_error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baozi_core::FormatId;
    use baozi_format_obj::{FORMAT_ID, ObjImporter};
    use baozi_import::{
        FormatSelection, ImportConfigOverlay, ImportOperationControl, ImportRequest,
        ImportRequestLayer, OptionCompileLimits, RegistryBuildLimits, RegistryBuilder,
        import_asset,
    };
    use baozi_io::{AssetPath, MemoryAssetIo};
    use baozi_operation::{LedgerEvidenceError, LedgerEvidencePurpose};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone)]
    struct CancelOnSecondPoll {
        polls: Arc<AtomicUsize>,
    }

    impl CancelOnSecondPoll {
        fn new() -> Self {
            Self {
                polls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn polls(&self) -> usize {
            self.polls.load(Ordering::Acquire)
        }
    }

    impl baozi_operation::CancellationSignal for CancelOnSecondPoll {
        fn is_cancelled(&self) -> bool {
            self.polls.fetch_add(1, Ordering::AcqRel) >= 1
        }
    }

    #[test]
    fn ledger_initialization_failure_returns_the_unchanged_scene_without_observation() {
        let scene = canonical_triangle_scene();
        let generation = scene.generation();
        let injected = LedgerLimitsError::Evidence {
            cause: LedgerEvidenceError::AllocationFailed {
                purpose: LedgerEvidencePurpose::Dimensions,
                requested_bytes: 128,
            },
        };

        let outcome = catch_unwind(AssertUnwindSafe(|| {
            match PostProcessRequests::empty().compile_with_control_and_ledger_limits(
                scene,
                PostProcessControl::default(),
                Err(injected),
            ) {
                Ok(_) => panic!("injected ledger initialization must fail"),
                Err(failure) => failure,
            }
        }));
        let failure = outcome.expect("ledger initialization failure must not unwind");
        let (scene, error) = failure.into_parts();

        assert!(matches!(
            error,
            PostProcessRunError::Initialization {
                source: crate::PostProcessInitializationError::ResourcePolicy
            }
        ));
        assert_eq!(scene.generation(), generation);
    }

    #[test]
    fn empty_execution_performs_a_distinct_final_control_poll() {
        let scene = canonical_triangle_scene();
        let generation = scene.generation();
        let cancellation = CancelOnSecondPoll::new();
        let observed = cancellation.clone();
        let control = PostProcessControl::default().with_cancellation_signal(cancellation);

        let compiled = PostProcessRequests::empty()
            .compile_with_control(scene, control)
            .expect("the first poll should admit compilation");
        let failure = compiled
            .execute()
            .expect_err("the final poll should observe cancellation");
        let (scene, failure) = failure.into_parts();

        assert_eq!(observed.polls(), 2);
        assert_eq!(scene.generation(), generation);
        assert!(matches!(failure.error(), PostProcessError::Control { .. }));
        assert_eq!(failure.resources().current_live_bytes(), 0);
        assert_eq!(failure.resources().committed_canonical_bytes(), 0);
    }

    #[test]
    fn finalization_failure_returns_the_original_scene_and_exact_resource_evidence() {
        let scene = canonical_triangle_scene();
        let generation = scene.generation();
        let mut compiled = PostProcessRequests::empty()
            .compile(scene)
            .expect("empty requests should compile");
        let reservation = compiled
            .coordinator
            .checkpoint()
            .reserve(
                PostProcessLedgerPhase::TransactionStaging,
                PostProcessDimension::StagingBytes,
                1,
            )
            .expect("test reservation should fit the default policy");

        let failure = compiled
            .execute()
            .expect_err("an active reservation must prevent clean finalization");
        let (scene, failure) = failure.into_parts();

        assert_eq!(scene.generation(), generation);
        assert!(matches!(
            failure.error(),
            PostProcessError::Finalization {
                cause: crate::PostProcessFinalizationError::OutstandingResources
            }
        ));
        assert_eq!(failure.cleanup_failure(), None);
        assert_eq!(failure.resources().current_live_bytes(), 1);
        assert_eq!(failure.resources().active_reservations(), 1);
        assert_eq!(failure.resources().committed_canonical_bytes(), 0);
        drop(reservation);
    }

    #[test]
    fn cleanup_failure_preserves_the_primary_control_cause() {
        let scene = canonical_triangle_scene();
        let generation = scene.generation();
        let cancellation = crate::PostProcessCancellationToken::new();
        let mut compiled = PostProcessRequests::empty()
            .compile_with_control(
                scene,
                PostProcessControl::default().with_cancellation(cancellation.clone()),
            )
            .expect("active control should admit compilation");
        let reservation = compiled
            .coordinator
            .checkpoint()
            .reserve(
                PostProcessLedgerPhase::TransactionStaging,
                PostProcessDimension::StagingBytes,
                1,
            )
            .expect("test reservation should fit the default policy");
        cancellation.cancel();

        let failure = compiled
            .execute()
            .expect_err("cancellation should stop execution");
        let (scene, failure) = failure.into_parts();

        assert_eq!(scene.generation(), generation);
        assert!(matches!(
            failure.error(),
            PostProcessError::Control {
                cause: crate::PostProcessControlCause::Cancelled {
                    origin: crate::PostProcessCancellationOrigin::Signal
                }
            }
        ));
        assert_eq!(
            failure.cleanup_failure(),
            Some(crate::PostProcessFinalizationError::OutstandingResources)
        );
        assert_eq!(failure.resources().current_live_bytes(), 1);
        assert_eq!(failure.resources().active_reservations(), 1);
        assert_eq!(failure.resources().committed_canonical_bytes(), 0);
        drop(reservation);
    }

    fn canonical_triangle_scene() -> ValidatedScene {
        let source = AssetPath::new("postprocess.obj").expect("static test path should be valid");
        let io = MemoryAssetIo::new();
        io.try_insert(
            source.clone(),
            Arc::<[u8]>::from(b"o postprocess\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".as_slice()),
        )
        .expect("test OBJ should fit the in-memory index");
        let mut registry = RegistryBuilder::new();
        registry
            .register(ObjImporter)
            .expect("OBJ importer registration should succeed");
        let registry = registry
            .freeze(RegistryBuildLimits::default())
            .expect("test registry should freeze");
        let request = ImportRequest::new().with_caller_layer(ImportRequestLayer::new(
            ImportConfigOverlay::default().with_format_selection(FormatSelection::Forced(
                FormatId::new(FORMAT_ID).expect("OBJ format id should be valid"),
            )),
        ));
        let request = registry
            .compile_request(&request, OptionCompileLimits::default())
            .expect("test request should compile");
        let report = import_asset(&request, &io, source, ImportOperationControl::default())
            .expect("test OBJ should import");
        report.into_parts().0
    }
}
