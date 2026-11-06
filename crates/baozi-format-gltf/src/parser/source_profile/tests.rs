use super::*;
use baozi_import::{
    CompiledOptions, DetectionOutcome, DetectionResult, DetectionScope, Format, FormatDescriptor,
    ImportCause, ImportConfigOverlay, ImportDimension, ImportFailure, ImportLossKind,
    ImportOperationControl, ImportRequest, ImportRequestLayer, ImportTerminalFailureClass,
    LossPolicy, MetadataOwnerRef, MetadataValueRef, NoFormatOptions, OptionCompileError,
    OptionCompileLimits, ParseContext, Probe, ReadConfidence, ReadHint, RegistryBuildLimits,
    RegistryBuildScope, RegistryBuilder, ResourceLimits, ResourceLimitsOverlay,
    SceneSnapshotLimits, ScopeAbortCause, SourceObjectLocatorRef,
};
use baozi_io::{AssetPath, MemoryAssetIo};
use baozi_operation::{CancellationToken, ControlCause};
use base64::Engine as _;
use std::{
    cell::RefCell,
    marker::PhantomData,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering as AtomicOrdering},
    },
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ComparisonSite {
    Direct,
    ExtrasSort,
    RunSort,
    RunDedup,
    DirectorySort,
    OwnerUnique,
    OrdinalTransfer,
}

struct CompletionProbe {
    site: ComparisonSite,
    completed: [usize; 7],
    cancel_on: Option<(ComparisonSite, usize, CancellationToken)>,
}

thread_local! {
    static COMPLETION_PROBE: RefCell<Option<CompletionProbe>> = const { RefCell::new(None) };
}

// Keep the test-only hook scoped to the invoking import thread, including unwind cleanup.
struct CompletionGuard(PhantomData<Rc<()>>);

impl CompletionGuard {
    fn new(cancel_on: Option<(ComparisonSite, usize, CancellationToken)>) -> Self {
        COMPLETION_PROBE.with(|slot| {
            let mut slot = slot.borrow_mut();
            assert!(slot.is_none());
            *slot = Some(CompletionProbe {
                site: ComparisonSite::Direct,
                completed: [0; 7],
                cancel_on,
            });
        });
        Self(PhantomData)
    }

    fn finish(self) -> [usize; 7] {
        COMPLETION_PROBE.with(|slot| slot.borrow_mut().take().unwrap().completed)
    }
}

impl Drop for CompletionGuard {
    fn drop(&mut self) {
        let probe = COMPLETION_PROBE.with(|slot| slot.borrow_mut().take());
        drop(probe);
    }
}

pub(super) fn comparison_scope(site: ComparisonSite) {
    COMPLETION_PROBE.with(|slot| {
        if let Some(probe) = slot.borrow_mut().as_mut() {
            probe.site = site;
        }
    });
}

pub(super) fn comparison_completed() {
    let site = COMPLETION_PROBE.with(|slot| slot.borrow().as_ref().map(|probe| probe.site));
    if let Some(site) = site {
        completed_at(site);
    }
}

pub(super) fn ordinal_transferred() {
    completed_at(ComparisonSite::OrdinalTransfer);
}

fn completed_at(completed_site: ComparisonSite) {
    let token = COMPLETION_PROBE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let probe = slot.as_mut()?;
        probe.completed[completed_site as usize] += 1;
        let (site, count, _) = probe.cancel_on.as_ref()?;
        if *site == completed_site && *count == probe.completed[completed_site as usize] {
            probe.cancel_on.take().map(|(_, _, token)| token)
        } else {
            None
        }
    });
    if let Some(token) = token {
        token.cancel();
    }
}

type ComparisonAction = for<'region, 'session> fn(
    &mut ParseRoot<'region, '_, 'session>,
    &AtomicUsize,
    &CancellationToken,
) -> Result<()>;

const PROBE_STOP: &str = "profile-comparison-probe-stop";

struct ComparisonProbe {
    action: ComparisonAction,
    physical: Arc<AtomicUsize>,
    token: CancellationToken,
}

impl Format for ComparisonProbe {
    type Options = NoFormatOptions;

    fn descriptor(&self) -> FormatDescriptor {
        crate::format_descriptor()
    }

    fn compile_defaults(
        &self,
        scope: &mut RegistryBuildScope<'_>,
    ) -> std::result::Result<CompiledOptions<Self::Options>, OptionCompileError> {
        NoFormatOptions::compile_defaults(scope)
    }

    fn detect(
        &self,
        _: &Probe<'_>,
        _: &ReadHint,
        _: &mut DetectionScope<'_, '_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, _: &Self::Options, context: &mut ParseContext<'_, '_>) -> ParseResult<()> {
        context.with_execution(
            |ctx| match (self.action)(ctx, &self.physical, &self.token) {
                Ok(()) => Err(ctx.format_failure(GltfFailure::internal(PROBE_STOP))),
                Err(ContentError::Scope(failure)) => Err(failure),
                Err(ContentError::Format(failure)) => Err(ctx.format_failure(failure)),
            },
        )
    }
}

struct Observation {
    failure: ImportFailure,
    physical: usize,
    completed: [usize; 7],
}

impl Observation {
    fn comparisons(&self) -> u64 {
        self.completed[..ComparisonSite::OrdinalTransfer as usize]
            .iter()
            .sum::<usize>() as u64
    }
}

fn run(
    action: ComparisonAction,
    max_work: Option<u64>,
    cancel_on: Option<(ComparisonSite, usize)>,
    primary: &[u8],
) -> Observation {
    let mut limits = ResourceLimits::default();
    if let Some(max_work) = max_work {
        limits.max_work_units = max_work;
    }
    run_with_assets(action, limits, cancel_on, primary, &[])
}

fn run_with_assets(
    action: ComparisonAction,
    limits: ResourceLimits,
    cancel_on: Option<(ComparisonSite, usize)>,
    primary: &[u8],
    assets: &[(&str, &[u8])],
) -> Observation {
    let physical = Arc::new(AtomicUsize::new(0));
    let token = CancellationToken::new();
    let guard = CompletionGuard::new(cancel_on.map(|(site, count)| (site, count, token.clone())));
    let mut registry = RegistryBuilder::new();
    registry
        .register(ComparisonProbe {
            action,
            physical: Arc::clone(&physical),
            token: token.clone(),
        })
        .unwrap();
    let request = ImportRequest::new().with_caller_layer(ImportRequestLayer::new(
        ImportConfigOverlay::default()
            .with_resource_limits(ResourceLimitsOverlay::from_limits(limits))
            .with_loss_policy(
                LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss),
            ),
    ));
    let plan = registry
        .freeze(RegistryBuildLimits::default())
        .unwrap()
        .plan(&request, OptionCompileLimits::default())
        .unwrap();
    let io = MemoryAssetIo::new();
    let path = AssetPath::new("profile.gltf").unwrap();
    io.try_insert(path.clone(), Arc::<[u8]>::from(primary))
        .unwrap();
    for &(path, bytes) in assets {
        io.try_insert(AssetPath::new(path).unwrap(), Arc::<[u8]>::from(bytes))
            .unwrap();
    }
    let failure = plan
        .read_asset(
            &io,
            path,
            ImportOperationControl::default().with_cancellation(token),
        )
        .expect_err("the probe stops before submission and retains the real parser ledger")
        .into_operation_failure()
        .unwrap();
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
    Observation {
        failure,
        physical: physical.load(AtomicOrdering::Relaxed),
        completed: guard.finish(),
    }
}

fn counts(failure: &ImportFailure) -> (u64, u64) {
    let row = failure
        .ledger()
        .dimension(ImportDimension::WorkUnits)
        .unwrap();
    (row.attempted(), row.consumed())
}

fn assert_cancelled(failure: &ImportFailure) {
    assert!(matches!(failure.cause(), ImportCause::Scope(abort)
        if matches!(abort.cause(), ScopeAbortCause::Control(ControlCause::Cancelled { .. }))));
}

fn assert_ledger_denial(failure: &ImportFailure) {
    match failure.cause() {
        ImportCause::Scope(abort) => {
            assert!(matches!(abort.cause(), ScopeAbortCause::Ledger(_)));
        }
        ImportCause::OperationTerminal(terminal) => {
            assert_eq!(terminal.class(), ImportTerminalFailureClass::Ledger);
        }
        cause => panic!("expected comparison admission failure, got {cause:?}"),
    }
}

fn baseline() -> u64 {
    let observation = run(|_, _, _| Ok(()), None, None, b"{}");
    assert!(matches!(
        observation.failure.cause(),
        ImportCause::Format(_)
    ));
    let (attempted, consumed) = counts(&observation.failure);
    assert_eq!(attempted, consumed);
    consumed
}

fn non_format_baseline() -> u64 {
    let observation = run(
        |ctx, _, token| {
            token.cancel();
            ctx.poll()?;
            Ok(())
        },
        None,
        None,
        b"{}",
    );
    assert_cancelled(&observation.failure);
    let (attempted, consumed) = counts(&observation.failure);
    assert_eq!(attempted, consumed);
    // Ledger/control stops do not construct the terminal format evidence in baseline().
    consumed
}

fn panic_baseline() -> u64 {
    let observation = run(
        |_, _, _| panic!("interrupt a comparison after admission but before normal return"),
        None,
        None,
        b"{}",
    );
    assert!(matches!(
        observation.failure.cause(),
        ImportCause::TrustedExtension {
            code: "importer-panicked",
            ..
        }
    ));
    let (attempted, consumed) = counts(&observation.failure);
    assert_eq!(attempted, consumed);
    consumed
}

fn ordinal_run<'session>(
    ctx: &mut ParseRoot<'_, '_, 'session>,
    ordinals: &[usize],
) -> Result<OwnerSourceRun<'session>> {
    let mut run = OwnerSourceRun::new(ctx)?;
    for &ordinal in ordinals {
        run.ordinals
            .try_push(&mut ctx.services()?, SelectedEntryOrdinal(ordinal))?;
    }
    run.owner = (!ordinals.is_empty()).then_some(ProfileOwner::Scene);
    Ok(run)
}

fn relation(profile: &SelectedSourceProfile<'_>) -> Vec<(ProfileOwner, SourceEntityId)> {
    profile
        .owners
        .as_slice()
        .iter()
        .flat_map(|range| {
            profile.ordinals.as_slice()[range.start..range.start + range.length]
                .iter()
                .map(move |ordinal| (range.owner, profile.entries()[ordinal.0].entity.id))
        })
        .collect()
}

fn owner_identities(
    ctx: &mut ParseRoot<'_, '_, '_>,
    profile: &SelectedSourceProfile<'_>,
    owner: ProfileOwner,
) -> Result<Vec<SourceEntityId>> {
    profile
        .for_owner(ctx, owner)?
        .iter()
        .map(|&ordinal| profile.source(ctx, ordinal).map(|source| source.entity.id))
        .collect()
}

fn numeric_comparisons(
    ctx: &mut ParseRoot<'_, '_, '_>,
    physical: &AtomicUsize,
    _: &CancellationToken,
) -> Result<()> {
    let _storage = ordinal_run(ctx, &[0; 16])?;
    let group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    ctx.child(&group, [0], |ledger_work| {
        let mut work = ProfileComparisonWork::new(ledger_work);
        for value in 0..8 {
            work.run(|| {
                physical.fetch_add(1, AtomicOrdering::Relaxed);
                value.cmp(&3)
            })?;
            assert_eq!(work.work.remaining(), [0]);
        }
        Ok(())
    })
}

#[test]
fn denied_current_comparison_never_runs_and_records_only_the_actual_prefix() {
    let before = non_format_baseline();
    for allowed in [0, 3] {
        let observation = run(numeric_comparisons, Some(before + allowed), None, b"{}");
        assert_ledger_denial(&observation.failure);
        assert_eq!(observation.physical as u64, allowed);
        assert_eq!(observation.comparisons(), allowed);
        assert_eq!(
            counts(&observation.failure),
            (before + allowed + 1, before + allowed)
        );
    }
    let actual_sort = run(order_long_run, Some(before + 3), None, b"{}");
    assert_ledger_denial(&actual_sort.failure);
    assert_eq!(actual_sort.completed[ComparisonSite::RunSort as usize], 3);
    assert_eq!(actual_sort.completed[ComparisonSite::RunDedup as usize], 0);
    assert_eq!(counts(&actual_sort.failure), (before + 4, before + 3));
}

#[test]
fn known_error_closes_idle_scope_and_unwind_does_not_complete_the_interrupted_comparison() {
    let format_before = baseline();
    let ordinary = run(
        |ctx, physical, _| {
            let _storage = ordinal_run(ctx, &[0; 16])?;
            let group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
            ctx.child(&group, [0], |ledger_work| {
                let mut work = ProfileComparisonWork::new(ledger_work);
                for _ in 0..3 {
                    work.run(|| physical.fetch_add(1, AtomicOrdering::Relaxed))?;
                }
                assert_eq!(work.work.remaining(), [0]);
                Err(ContentError::Format(GltfFailure::internal(PROBE_STOP)))
            })
        },
        None,
        None,
        b"{}",
    );
    assert!(matches!(ordinary.failure.cause(), ImportCause::Format(_)));
    assert_eq!(ordinary.physical, 3);
    assert_eq!(ordinary.comparisons(), 3);
    assert_eq!(
        counts(&ordinary.failure),
        (format_before + 3, format_before + 3)
    );

    let interrupted = run(
        |ctx, physical, _| {
            let _storage = ordinal_run(ctx, &[0; 16])?;
            let group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
            ctx.child(&group, [0], |ledger_work| {
                let mut work = ProfileComparisonWork::new(ledger_work);
                for _ in 0..3 {
                    work.run(|| physical.fetch_add(1, AtomicOrdering::Relaxed))?;
                }
                work.run::<()>(|| {
                    physical.fetch_add(1, AtomicOrdering::Relaxed);
                    panic!("interrupt a comparison after admission but before normal return");
                })?;
                Ok(())
            })
        },
        None,
        None,
        b"{}",
    );
    assert!(matches!(
        interrupted.failure.cause(),
        ImportCause::TrustedExtension {
            code: "importer-panicked",
            ..
        }
    ));
    assert_eq!(interrupted.physical, 4);
    assert_eq!(interrupted.comparisons(), 3);
    let before = panic_baseline();
    assert_eq!(counts(&interrupted.failure), (before + 4, before + 3));
}

#[test]
fn completed_runs_preserve_the_literal_relation_and_reuse_charged_scratch() {
    let observation = run(
        |ctx, physical, _| {
            let map = GltfSourceMap::from_primary_for_test(ctx)?;
            let mut selected = SelectedSourceProfile::from_source_map(ctx, &map, |_| true)?;
            let mut ordinary = OwnerSourceRun::new(ctx)?;
            let suspended = OwnerSourceRun::new(ctx)?;
            let primitive_a = entity(SourceEntityKind::Primitive, Some(0), 1);
            let primitive_b = entity(SourceEntityKind::Primitive, Some(1), 0);
            let buffer = entity(SourceEntityKind::Buffer, None, 2);
            let view = entity(SourceEntityKind::BufferView, None, 1);
            let accessor = entity(SourceEntityKind::Accessor, None, 1);
            let expected = [
                (ProfileOwner::Scene, DOCUMENT),
                (ProfileOwner::SyntheticRoot, DOCUMENT),
                (ProfileOwner::Attribute(0), primitive_a),
                (ProfileOwner::Attribute(0), primitive_b),
                (ProfileOwner::Attribute(0), buffer),
                (ProfileOwner::Attribute(0), view),
                (ProfileOwner::Attribute(0), accessor),
                (ProfileOwner::Attribute(1), accessor),
            ];
            for (owner, candidates) in [
                (ProfileOwner::Attribute(1), &[accessor, accessor][..]),
                (ProfileOwner::Attribute(0), &[accessor, view, buffer, primitive_b, primitive_a, view, primitive_a][..]),
                (ProfileOwner::SyntheticRoot, &[DOCUMENT][..]),
                (ProfileOwner::Scene, &[DOCUMENT][..]),
            ] {
                for &source in candidates {
                    selected.link(ctx, &mut ordinary, owner, source)?;
                }
                let capacity = ordinary.ordinals.capacity();
                selected.complete_owner(ctx, &mut ordinary, owner)?;
                assert_eq!(ordinary.ordinals.capacity(), capacity);
                assert!(ordinary.ordinals.is_empty());
                assert_eq!(ordinary.owner, None);
            }
            assert!(selected.for_owner(ctx, ProfileOwner::Scene).is_err());
            selected.finish(ctx, &ordinary, &suspended)?;
            assert_eq!(relation(&selected), expected);
            assert_eq!(selected.link_count(), expected.len());
            assert_eq!(selected.owners.len(), 4);
            assert_eq!(
                owner_identities(ctx, &selected, ProfileOwner::Attribute(0))?,
                [primitive_a, primitive_b, buffer, view, accessor]
            );
            assert_eq!(
                owner_identities(ctx, &selected, ProfileOwner::Scene)?,
                [DOCUMENT]
            );
            physical.store(1, AtomicOrdering::Relaxed);
            Ok(())
        },
        None,
        None,
        br#"{"meshes":[{"primitives":[{},{}]},{"primitives":[{}]}],"buffers":[{},{},{}],"bufferViews":[{},{}],"accessors":[{},{}]}"#,
    );
    assert!(matches!(
        observation.failure.cause(),
        ImportCause::Format(_)
    ));
    assert_eq!(observation.physical, 1);
    assert_eq!(
        observation.completed[ComparisonSite::OrdinalTransfer as usize],
        8
    );
    assert!(observation.completed[ComparisonSite::RunSort as usize] > 0);
    assert!(observation.completed[ComparisonSite::DirectorySort as usize] > 0);
    assert_eq!(
        observation.completed[ComparisonSite::OwnerUnique as usize],
        3
    );
    let (attempted, consumed) = counts(&observation.failure);
    assert_eq!(attempted, consumed);
}

fn order_long_run(
    ctx: &mut ParseRoot<'_, '_, '_>,
    _: &AtomicUsize,
    _: &CancellationToken,
) -> Result<()> {
    let mut run = ordinal_run(ctx, &[0; 130])?;
    let capacity = run.ordinals.capacity();
    let result = run.order(ctx);
    assert!(result.is_err());
    assert_eq!(
        run.ordinals.len(),
        130,
        "cancelled dedup must not truncate its suffix"
    );
    assert_eq!(run.ordinals.capacity(), capacity);
    result
}

fn order_short_run(
    ctx: &mut ParseRoot<'_, '_, '_>,
    _: &AtomicUsize,
    _: &CancellationToken,
) -> Result<()> {
    let mut run = ordinal_run(ctx, &[0; 2])?;
    let capacity = run.ordinals.capacity();
    let result = run.order(ctx);
    assert!(result.is_err());
    assert_eq!(
        run.ordinals.len(),
        2,
        "scope-exit cancellation precedes truncation"
    );
    assert_eq!(run.ordinals.capacity(), capacity);
    result
}

#[test]
fn actual_sort_and_dedup_cancel_within_64_comparisons_and_at_short_scope_exits() {
    let before = non_format_baseline();
    for (action, at_poll) in [
        (order_long_run as ComparisonAction, 64),
        (order_short_run as ComparisonAction, 1),
    ] {
        for site in [ComparisonSite::RunSort, ComparisonSite::RunDedup] {
            let observation = run(action, None, Some((site, 1)), b"{}");
            assert_cancelled(&observation.failure);
            assert_eq!(observation.completed[site as usize], at_poll);
            if site == ComparisonSite::RunSort {
                assert_eq!(observation.completed[ComparisonSite::RunDedup as usize], 0);
            }
            let expected = before + observation.comparisons();
            assert_eq!(counts(&observation.failure), (expected, expected));
        }
    }
}

#[test]
fn empty_singleton_order_has_no_comparison_work_and_still_observes_entry_cancellation() {
    let before = baseline();
    let observation = run(
        |ctx, _, _| {
            for input in [&[][..], &[0][..]] {
                let mut run = ordinal_run(ctx, input)?;
                let capacity = run.ordinals.capacity();
                run.order(ctx)?;
                assert_eq!(run.ordinals.len(), input.len());
                assert_eq!(run.ordinals.capacity(), capacity);
            }
            Ok(())
        },
        None,
        None,
        b"{}",
    );
    assert!(matches!(
        observation.failure.cause(),
        ImportCause::Format(_)
    ));
    assert_eq!(observation.comparisons(), 0);
    assert_eq!(counts(&observation.failure), (before, before));

    fn cancel_empty(
        ctx: &mut ParseRoot<'_, '_, '_>,
        _: &AtomicUsize,
        token: &CancellationToken,
    ) -> Result<()> {
        let mut run = ordinal_run(ctx, &[])?;
        token.cancel();
        run.order(ctx)
    }
    fn cancel_singleton(
        ctx: &mut ParseRoot<'_, '_, '_>,
        _: &AtomicUsize,
        token: &CancellationToken,
    ) -> Result<()> {
        let mut run = ordinal_run(ctx, &[0])?;
        token.cancel();
        run.order(ctx)
    }
    let before = non_format_baseline();
    for action in [cancel_empty as ComparisonAction, cancel_singleton] {
        let observation = run(action, None, None, b"{}");
        assert_cancelled(&observation.failure);
        assert_eq!(observation.comparisons(), 0);
        assert_eq!(counts(&observation.failure), (before, before));
    }
}

#[test]
fn duplicate_completed_owners_and_unfinished_runs_never_authorize_lookup() {
    // A rejected parser child closes its execution channel. Each independent fault
    // needs a fresh import, and its original error must reach the format boundary.
    for (case, unique_comparisons) in [
        ("unfinished", 0),
        ("unfinished-lookup", 0),
        ("duplicate", 1),
    ] {
        let primary = serde_json::to_vec(&serde_json::json!({"case": case})).unwrap();
        let observation = run(
            |ctx, physical, _| {
                let case: serde_json::Value = serde_json::from_slice(ctx.primary_bytes()).unwrap();
                let map = GltfSourceMap::from_primary_for_test(ctx)?;
                let mut selected = SelectedSourceProfile::from_source_map(ctx, &map, |_| true)?;
                let mut ordinary = OwnerSourceRun::new(ctx)?;
                let mut suspended = OwnerSourceRun::new(ctx)?;
                selected.link(ctx, &mut suspended, ProfileOwner::Sampler(0), DOCUMENT)?;
                let error = match case["case"].as_str().unwrap() {
                    "unfinished" => selected
                        .finish(ctx, &ordinary, &suspended)
                        .expect_err("a suspended owner prevents finalization"),
                    "unfinished-lookup" => {
                        selected.link(ctx, &mut ordinary, ProfileOwner::Scene, DOCUMENT)?;
                        selected.complete_owner(ctx, &mut ordinary, ProfileOwner::Scene)?;
                        selected
                            .for_owner(ctx, ProfileOwner::Scene)
                            .expect_err("an unfinished directory never authorizes lookup")
                    }
                    "duplicate" => {
                        selected.complete_owner(ctx, &mut suspended, ProfileOwner::Sampler(0))?;
                        for _ in 0..2 {
                            selected.link(ctx, &mut ordinary, ProfileOwner::Scene, DOCUMENT)?;
                            selected.complete_owner(ctx, &mut ordinary, ProfileOwner::Scene)?;
                        }
                        selected
                            .finish(ctx, &ordinary, &suspended)
                            .expect_err("a completed owner may not recur")
                    }
                    _ => panic!("known owner-rejection fixture"),
                };
                assert!(!selected.finished);
                physical.store(1, AtomicOrdering::Relaxed);
                Err(error)
            },
            None,
            None,
            &primary,
        );
        assert!(
            matches!(observation.failure.cause(), ImportCause::Format(_)),
            "{case}: {:?}",
            observation.failure.cause(),
        );
        assert_eq!(observation.physical, 1, "{case}");
        assert_eq!(
            observation.completed[ComparisonSite::OwnerUnique as usize],
            unique_comparisons,
            "{case}",
        );
    }
}

#[test]
fn invalid_ranges_uncovered_transfers_and_unadmitted_ordinals_are_rejected() {
    for case in [
        "empty-range",
        "gap-range",
        "oversized-range",
        "overflow-range",
        "uncovered-ordinals",
        "overlapping-ranges",
        "invalid-transfer",
        "invalid-source",
        "invalid-translation",
        "usize-source-index-overflow",
        "u32-source-index-overflow",
    ] {
        let primary = serde_json::to_vec(&serde_json::json!({
            "case": case,
            "nodes": [{}, {}],
        }))
        .unwrap();
        let observation = run(
            |ctx, physical, _| {
                let case: serde_json::Value = serde_json::from_slice(ctx.primary_bytes()).unwrap();
                let case = case["case"].as_str().unwrap();
                let map = GltfSourceMap::from_primary_for_test(ctx)?;
                let mut selected = SelectedSourceProfile::from_source_map(ctx, &map, |_| true)?;
                let ordinary = OwnerSourceRun::new(ctx)?;
                let suspended = OwnerSourceRun::new(ctx)?;
                let error = match case {
                    "empty-range" | "gap-range" | "oversized-range" | "overflow-range" => {
                        let (start, length) = match case {
                            "empty-range" => (0, 0),
                            "gap-range" => (1, 1),
                            "oversized-range" => (0, 2),
                            _ => (usize::MAX, 1),
                        };
                        selected
                            .ordinals
                            .try_push(&mut ctx.services()?, SelectedEntryOrdinal(0))?;
                        selected.owners.try_push(
                            &mut ctx.services()?,
                            OwnerSourceRange {
                                owner: ProfileOwner::Scene,
                                start,
                                length,
                            },
                        )?;
                        selected
                            .finish(ctx, &ordinary, &suspended)
                            .expect_err("invalid retained owner range")
                    }
                    "uncovered-ordinals" => {
                        selected
                            .ordinals
                            .try_push(&mut ctx.services()?, SelectedEntryOrdinal(0))?;
                        selected
                            .finish(ctx, &ordinary, &suspended)
                            .expect_err("every retained ordinal belongs to a completed owner")
                    }
                    "overlapping-ranges" => {
                        selected.ordinals.try_extend_from_slice(
                            &mut ctx.services()?,
                            &[SelectedEntryOrdinal(0); 2],
                        )?;
                        for owner in [ProfileOwner::Scene, ProfileOwner::SyntheticRoot] {
                            selected.owners.try_push(
                                &mut ctx.services()?,
                                OwnerSourceRange {
                                    owner,
                                    start: 0,
                                    length: 1,
                                },
                            )?;
                        }
                        selected
                            .finish(ctx, &ordinary, &suspended)
                            .expect_err("retained owner ranges may not overlap")
                    }
                    "invalid-transfer" => {
                        let mut invalid = ordinal_run(ctx, &[usize::MAX])?;
                        let error = selected
                            .complete_owner(ctx, &mut invalid, ProfileOwner::Scene)
                            .expect_err("only admitted ordinals can transfer");
                        assert!(selected.ordinals.is_empty());
                        assert!(selected.owners.is_empty());
                        error
                    }
                    "invalid-source" => selected
                        .source(ctx, SelectedEntryOrdinal(usize::MAX))
                        .map(|_| ())
                        .expect_err("only admitted ordinals resolve to sources"),
                    "invalid-translation" => selected
                        .ordinal_source_index(ctx, 1, SelectedEntryOrdinal(usize::MAX))
                        .expect_err("only admitted ordinals translate"),
                    "usize-source-index-overflow" | "u32-source-index-overflow" => {
                        let ordinal = selected
                            .required_ordinal(ctx, entity(SourceEntityKind::Node, None, 1))?;
                        let resource_count = if case == "usize-source-index-overflow" {
                            usize::MAX
                        } else {
                            u32::MAX as usize
                        };
                        selected
                            .ordinal_source_index(ctx, resource_count, ordinal)
                            .expect_err("translated source indices stay representable")
                    }
                    _ => panic!("known owner-range rejection fixture"),
                };
                assert!(!selected.finished);
                physical.store(1, AtomicOrdering::Relaxed);
                Err(error)
            },
            None,
            None,
            &primary,
        );
        assert!(
            matches!(observation.failure.cause(), ImportCause::Format(_)),
            "{case}: {:?}",
            observation.failure.cause(),
        );
        assert_eq!(observation.physical, 1, "{case}");
        assert_eq!(
            observation.completed[ComparisonSite::OrdinalTransfer as usize],
            0,
            "{case}",
        );
    }
}

fn pending_nodes<'session>(
    ctx: &mut ParseRoot<'_, '_, 'session>,
) -> Result<(SelectedSourceProfile<'session>, OwnerSourceRun<'session>)> {
    let map = GltfSourceMap::from_primary_for_test(ctx)?;
    let selected = SelectedSourceProfile::from_source_map(ctx, &map, |_| true)?;
    let mut pending = OwnerSourceRun::new(ctx)?;
    for source in selected
        .entries()
        .iter()
        .filter(|source| source.entity.id != DOCUMENT)
    {
        selected.link(ctx, &mut pending, ProfileOwner::Scene, source.entity.id)?;
    }
    Ok((selected, pending))
}

fn transfer_nodes(
    ctx: &mut ParseRoot<'_, '_, '_>,
    _: &AtomicUsize,
    _: &CancellationToken,
) -> Result<()> {
    let (mut selected, mut pending) = pending_nodes(ctx)?;
    let length = pending.ordinals.len();
    let capacity = pending.ordinals.capacity();
    let result = selected.complete_owner(ctx, &mut pending, ProfileOwner::Scene);
    assert!(result.is_err());
    assert_eq!(selected.ordinals.len(), length.min(64));
    assert!(selected.owners.is_empty());
    assert!(!selected.finished);
    assert_eq!(pending.ordinals.len(), length);
    assert_eq!(pending.ordinals.capacity(), capacity);
    result
}

#[test]
fn ordinal_transfer_cancellation_retains_only_its_actual_prefix_without_a_descriptor() {
    for length in [1, 130] {
        let primary = serde_json::to_vec(
            &serde_json::json!({ "nodes": vec![serde_json::json!({}); length] }),
        )
        .unwrap();
        let setup = run(
            |ctx, _, token| {
                let _pending = pending_nodes(ctx)?;
                token.cancel();
                ctx.poll()?;
                Ok(())
            },
            None,
            None,
            &primary,
        );
        assert_cancelled(&setup.failure);
        let observation = run(
            transfer_nodes,
            None,
            Some((ComparisonSite::OrdinalTransfer, 1)),
            &primary,
        );
        assert_cancelled(&observation.failure);
        let transferred = length.min(64) as u64;
        assert_eq!(
            observation.completed[ComparisonSite::OrdinalTransfer as usize] as u64,
            transferred
        );
        let expected = counts(&setup.failure).1 + 2 + observation.comparisons() + transferred;
        assert_eq!(counts(&observation.failure), (expected, expected));
        if length > 64 {
            let limit = counts(&setup.failure).1 + 2 + observation.comparisons() + 3;
            let denied = run(
                |ctx, physical, token| {
                    let (mut selected, mut pending) = pending_nodes(ctx)?;
                    let error = selected
                        .complete_owner(ctx, &mut pending, ProfileOwner::Scene)
                        .expect_err("the next transfer must be denied before it runs");
                    physical.store(selected.ordinals.len(), AtomicOrdering::Relaxed);
                    assert!(selected.owners.is_empty());
                    assert!(!selected.finished);
                    token.cancel();
                    assert_eq!(ctx.poll().unwrap_err().to_string(), error.to_string());
                    Err(error)
                },
                Some(limit),
                None,
                &primary,
            );
            assert_ledger_denial(&denied.failure);
            assert_eq!(denied.physical, 3);
            assert_eq!(
                denied.completed[ComparisonSite::OrdinalTransfer as usize],
                3
            );
            assert_eq!(counts(&denied.failure), (limit + 1, limit));
        }
    }
}

fn wide_reserved_document_run<'session>(
    ctx: &mut ParseRoot<'_, '_, 'session>,
) -> Result<(SelectedSourceProfile<'session>, OwnerSourceRun<'session>)> {
    let map = GltfSourceMap::from_primary_for_test(ctx)?;
    let selected = SelectedSourceProfile::from_source_map(ctx, &map, |_| true)?;
    drop(map);
    let mut pending = OwnerSourceRun::new(ctx)?;
    pending
        .ordinals
        .try_reserve(&mut ctx.services()?, 16 * 1024)?;
    selected.link(ctx, &mut pending, ProfileOwner::Scene, DOCUMENT)?;
    assert_eq!(pending.ordinals.len(), 1);
    assert!(pending.ordinals.capacity() >= 16 * 1024);
    assert_eq!(selected.ordinals.capacity(), 0);
    assert_eq!(selected.owners.capacity(), 0);
    Ok((selected, pending))
}

#[test]
fn directory_storage_denial_follows_ordinal_reservation_before_any_transfer() {
    let reserved = run(
        |ctx, physical, token| {
            let (mut selected, _pending) = wide_reserved_document_run(ctx)?;
            selected.ordinals.try_reserve(&mut ctx.services()?, 1)?;
            physical.store(
                selected.ordinals.capacity() * std::mem::size_of::<SelectedEntryOrdinal>(),
                AtomicOrdering::Relaxed,
            );
            token.cancel();
            ctx.poll()?;
            Ok(())
        },
        None,
        None,
        b"{}",
    );
    assert_cancelled(&reserved.failure);
    assert!(reserved.physical > 0);
    assert_eq!(
        reserved.completed[ComparisonSite::OrdinalTransfer as usize],
        0
    );
    let limit = reserved.failure.ledger().peak_live_bytes();

    let completed = run(
        |ctx, physical, token| {
            let (mut selected, mut pending) = wide_reserved_document_run(ctx)?;
            let capacity = pending.ordinals.capacity();
            selected.complete_owner(ctx, &mut pending, ProfileOwner::Scene)?;
            assert_eq!(selected.ordinals.as_slice(), &[SelectedEntryOrdinal(0)]);
            assert_eq!(selected.owners.len(), 1);
            assert!(!selected.finished);
            assert!(pending.ordinals.is_empty());
            assert_eq!(pending.ordinals.capacity(), capacity);
            physical.store(
                selected.owners.capacity() * std::mem::size_of::<OwnerSourceRange>(),
                AtomicOrdering::Relaxed,
            );
            token.cancel();
            ctx.poll()?;
            Ok(())
        },
        None,
        None,
        b"{}",
    );
    assert_cancelled(&completed.failure);
    assert_eq!(
        completed.completed[ComparisonSite::OrdinalTransfer as usize],
        1
    );
    assert!(completed.physical > 0);
    assert_eq!(
        completed.failure.ledger().peak_live_bytes(),
        limit + completed.physical as u64,
        "the measured frontier adds only the directory's actual retained capacity",
    );

    let mut limits = ResourceLimits::default();
    limits.max_live_bytes = limit;
    let denied = run_with_assets(
        |ctx, physical, token| {
            let (mut selected, mut pending) = wide_reserved_document_run(ctx)?;
            let capacity = pending.ordinals.capacity();
            let error = selected
                .complete_owner(ctx, &mut pending, ProfileOwner::Scene)
                .expect_err("directory storage must be denied after ordinal storage is admitted");
            assert!(matches!(error, ContentError::Scope(_)));
            assert!(selected.ordinals.capacity() > 0);
            assert!(selected.ordinals.is_empty());
            assert_eq!(selected.owners.capacity(), 0);
            assert!(selected.owners.is_empty());
            assert!(!selected.finished);
            assert_eq!(pending.owner, Some(ProfileOwner::Scene));
            assert_eq!(pending.ordinals.as_slice(), &[SelectedEntryOrdinal(0)]);
            assert_eq!(pending.ordinals.capacity(), capacity);
            physical.store(1, AtomicOrdering::Relaxed);
            token.cancel();
            Err(ctx
                .poll()
                .expect_err("the first storage denial must remain terminal")
                .into())
        },
        limits,
        None,
        b"{}",
        &[],
    );
    assert_eq!(denied.physical, 1);
    assert_eq!(
        denied.completed[ComparisonSite::OrdinalTransfer as usize],
        0
    );
    assert_eq!(denied.failure.ledger().peak_live_bytes(), limit);
    let ImportCause::Scope(abort) = denied.failure.cause() else {
        panic!("the storage denial must survive the later cancellation poll");
    };
    assert!(matches!(
        abort.cause(),
        ScopeAbortCause::Ledger(baozi_operation::LedgerCause::LimitExceeded {
            class: baozi_operation::CounterClass::Live,
            dimension: ImportDimension::ScratchBytes,
            value,
            limit: denied_limit,
        }) if *denied_limit == limit && *value == limit + completed.physical as u64
    ));
}

#[test]
fn empty_directory_finalization_observes_entry_cancellation() {
    let observation = run(
        |ctx, _, token| {
            let map = GltfSourceMap::from_primary_for_test(ctx)?;
            let mut selected = SelectedSourceProfile::from_source_map(ctx, &map, |_| true)?;
            let ordinary = OwnerSourceRun::new(ctx)?;
            let suspended = OwnerSourceRun::new(ctx)?;
            token.cancel();
            let result = selected.finish(ctx, &ordinary, &suspended);
            assert!(result.is_err());
            assert!(!selected.finished);
            result
        },
        None,
        None,
        b"{}",
    );
    assert_cancelled(&observation.failure);
    assert_eq!(observation.comparisons(), 0);
}

fn completed_nodes<'session>(
    ctx: &mut ParseRoot<'_, '_, 'session>,
) -> Result<(
    SelectedSourceProfile<'session>,
    OwnerSourceRun<'session>,
    OwnerSourceRun<'session>,
)> {
    let map = GltfSourceMap::from_primary_for_test(ctx)?;
    let mut selected = SelectedSourceProfile::from_source_map(ctx, &map, |_| true)?;
    let mut ordinary = OwnerSourceRun::new(ctx)?;
    let suspended = OwnerSourceRun::new(ctx)?;
    for index in (1..selected.entries.len()).rev() {
        let source = selected.entries()[index].entity.id;
        let owner = ProfileOwner::Node(baozi_import::authoring::NodeLocalId::new(
            source.source_index as u32,
        ));
        selected.link(ctx, &mut ordinary, owner, source)?;
        selected.complete_owner(ctx, &mut ordinary, owner)?;
    }
    Ok((selected, ordinary, suspended))
}

#[test]
fn directory_ordering_and_uniqueness_observe_bounded_and_short_scope_cancellation() {
    for length in [2, 130] {
        let primary = serde_json::to_vec(
            &serde_json::json!({ "nodes": vec![serde_json::json!({}); length] }),
        )
        .unwrap();
        let setup = run(
            |ctx, _, token| {
                let _completed = completed_nodes(ctx)?;
                token.cancel();
                ctx.poll()?;
                Ok(())
            },
            None,
            None,
            &primary,
        );
        assert_cancelled(&setup.failure);
        for site in [ComparisonSite::DirectorySort, ComparisonSite::OwnerUnique] {
            let observation = run(
                |ctx, _, _| {
                    let (mut selected, ordinary, suspended) = completed_nodes(ctx)?;
                    let result = selected.finish(ctx, &ordinary, &suspended);
                    assert!(result.is_err());
                    assert!(!selected.finished);
                    result
                },
                None,
                Some((site, 1)),
                &primary,
            );
            assert_cancelled(&observation.failure);
            assert_eq!(
                observation.completed[site as usize],
                if length == 2 { 1 } else { 64 }
            );
            let expected = counts(&setup.failure).1 + length as u64 + 2 + observation.comparisons();
            assert_eq!(counts(&observation.failure), (expected, expected));
        }
    }
}

#[test]
fn candidate_membership_precedes_accessor_and_view_dependency_dereferences() {
    let observation = run(
        |ctx, physical, _| {
            let surfaces = crate::admission::admit(ctx)?;
            let document = crate::document::Document::decode(ctx, surfaces)?;
            for missing in [SourceEntityKind::Accessor, SourceEntityKind::BufferView, SourceEntityKind::Buffer] {
                let selected = SelectedSourceProfile::from_source_map(ctx, document.source_map(), |id| id.kind != missing)?;
                let mut pending = OwnerSourceRun::new(ctx)?;
                let error = link_accessor(ctx, &selected, &mut pending, document.tables(), ProfileOwner::Primitive(0), 0)
                    .expect_err("every candidate must resolve against the selected inventory");
                assert_eq!(error.to_string(), "selected glTF owner has no admitted source identity");
                assert_eq!(pending.ordinals.len(), match missing {
                    SourceEntityKind::Accessor => 0,
                    SourceEntityKind::BufferView => 1,
                    SourceEntityKind::Buffer => 2,
                    _ => unreachable!(),
                });
            }
            let selected = SelectedSourceProfile::from_source_map(ctx, document.source_map(), |_| true)?;
            let mut pending = OwnerSourceRun::new(ctx)?;
            for result in [
                link_accessor(ctx, &selected, &mut pending, document.tables(), ProfileOwner::Primitive(0), 1),
                link_view(ctx, &selected, &mut pending, document.tables(), ProfileOwner::Primitive(0), 1),
            ] {
                assert_eq!(result.err().unwrap().to_string(), "selected glTF owner has no admitted source identity");
            }
            assert!(pending.ordinals.is_empty());
            physical.store(1, AtomicOrdering::Relaxed);
            Ok(())
        },
        None,
        None,
        br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":12,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAA"}],"bufferViews":[{"buffer":0,"byteLength":12}],"accessors":[{"bufferView":0,"componentType":5126,"count":1,"type":"VEC3"}]}"#,
    );
    assert_eq!(observation.physical, 1);
}

#[test]
fn shared_primitive_relations_match_source_derived_counts_and_complete_owner_order() {
    for (count, expected_links) in [(1, 32), (8, 186), (32, 714)] {
        let mut binary = [
            0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0,
            0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0,
        ]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>();
        for index in [0_u16, 1, 2] {
            binary.extend_from_slice(&index.to_le_bytes());
        }
        let primary = serde_json::to_vec(&serde_json::json!({
            "asset": {"version": "2.0"}, "scene": 0,
            "scenes": [{"nodes": [0, 1]}], "nodes": [{"mesh": 0}, {"mesh": 0}],
            "meshes": [{"primitives": vec![serde_json::json!({"attributes": {"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}, "indices": 3}); count]}],
            "buffers": [{"byteLength": 102, "uri": format!("data:application/octet-stream;base64,{}", base64::engine::general_purpose::STANDARD.encode(binary))}],
            "bufferViews": [
                {"buffer": 0, "byteLength": 36}, {"buffer": 0, "byteOffset": 36, "byteLength": 36},
                {"buffer": 0, "byteOffset": 72, "byteLength": 24}, {"buffer": 0, "byteOffset": 96, "byteLength": 6}
            ],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0]},
                {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3"},
                {"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC2"},
                {"bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR"}
            ]
        })).unwrap();
        let observation = run(
            |ctx, physical, _| {
                use baozi_import::authoring::NodeLocalId;
                let model =
                    super::super::read_normalized_gltf(ctx, &crate::GltfImportOptions::default())?;
                let profile = &model.sources;
                assert_eq!(model.geometry.logical_meshes.len(), 1);
                assert_eq!(model.nodes.mesh_instances.len(), 2);
                let primitives = model.geometry.logical_meshes.as_slice()[0]
                    .primitives
                    .as_slice();
                let mut expected = vec![
                    (
                        ProfileOwner::Scene,
                        entity(SourceEntityKind::Scene, None, 0),
                    ),
                    (
                        ProfileOwner::SyntheticRoot,
                        entity(SourceEntityKind::Scene, None, 0),
                    ),
                    (
                        ProfileOwner::Node(NodeLocalId::new(0)),
                        entity(SourceEntityKind::Node, None, 0),
                    ),
                    (
                        ProfileOwner::Node(NodeLocalId::new(1)),
                        entity(SourceEntityKind::Node, None, 1),
                    ),
                    (ProfileOwner::Material(0), DOCUMENT),
                    (
                        ProfileOwner::LogicalMesh(0),
                        entity(SourceEntityKind::Mesh, None, 0),
                    ),
                ];
                for (index, primitive) in primitives.iter().enumerate() {
                    assert_eq!(primitive.attributes.len(), 3);
                    let owner = ProfileOwner::Primitive(index as u32);
                    expected.extend([
                        (owner, entity(SourceEntityKind::Primitive, Some(0), index)),
                        (owner, entity(SourceEntityKind::Buffer, None, 0)),
                        (owner, entity(SourceEntityKind::BufferView, None, 0)),
                        (owner, entity(SourceEntityKind::BufferView, None, 1)),
                        (owner, entity(SourceEntityKind::BufferView, None, 2)),
                        (owner, entity(SourceEntityKind::BufferView, None, 3)),
                        (owner, entity(SourceEntityKind::Accessor, None, 0)),
                        (owner, entity(SourceEntityKind::Accessor, None, 1)),
                        (owner, entity(SourceEntityKind::Accessor, None, 2)),
                        (owner, entity(SourceEntityKind::Accessor, None, 3)),
                    ]);
                }
                for index in 0..primitives.len() {
                    for semantic in 0..3 {
                        let owner = ProfileOwner::Attribute((index * 3 + semantic) as u32);
                        expected.extend([
                            (owner, entity(SourceEntityKind::Primitive, Some(0), index)),
                            (owner, entity(SourceEntityKind::Buffer, None, 0)),
                            (owner, entity(SourceEntityKind::BufferView, None, semantic)),
                            (owner, entity(SourceEntityKind::Accessor, None, semantic)),
                        ]);
                    }
                }
                expected.extend([
                    (
                        ProfileOwner::MeshInstance(0),
                        entity(SourceEntityKind::Node, None, 0),
                    ),
                    (
                        ProfileOwner::MeshInstance(0),
                        entity(SourceEntityKind::Mesh, None, 0),
                    ),
                    (
                        ProfileOwner::MeshInstance(1),
                        entity(SourceEntityKind::Node, None, 1),
                    ),
                    (
                        ProfileOwner::MeshInstance(1),
                        entity(SourceEntityKind::Mesh, None, 0),
                    ),
                ]);
                assert_eq!(relation(profile), expected);
                let translated = profile
                    .for_owner(ctx, ProfileOwner::Primitive(0))?
                    .iter()
                    .map(|&ordinal| profile.ordinal_source_index(ctx, 1, ordinal))
                    .collect::<Result<Vec<_>>>()?;
                let count = primitives.len() as u32;
                assert_eq!(
                    translated,
                    [
                        5,
                        count + 5,
                        count + 6,
                        count + 7,
                        count + 8,
                        count + 9,
                        count + 10,
                        count + 11,
                        count + 12,
                        count + 13
                    ]
                );
                physical.store(profile.link_count(), AtomicOrdering::Relaxed);
                Ok(())
            },
            None,
            None,
            &primary,
        );
        assert_eq!(observation.physical, expected_links);
        assert_eq!(
            observation.completed[ComparisonSite::OrdinalTransfer as usize],
            expected_links
        );
    }
}

#[test]
fn sparse_owner_relations_preserve_embedded_buffer_identities_through_authoring() {
    let mut binary = vec![0; 36];
    binary.extend_from_slice(&[1, 2, 0, 0]);
    for value in [1.0_f32, 0.0, 0.0, 0.0, 1.0, 0.0] {
        binary.extend_from_slice(&value.to_le_bytes());
    }
    let buffer = serde_json::json!({
        "byteLength": 64,
        "uri": format!(
            "data:application/octet-stream;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&binary),
        ),
    });
    let mut registry = RegistryBuilder::new();
    registry.register(crate::GltfImporter).unwrap();
    let plan = registry
        .freeze(RegistryBuildLimits::default())
        .unwrap()
        .plan(&ImportRequest::new(), OptionCompileLimits::default())
        .unwrap();
    for (buffer_count, expected_sources) in [
        (1, &[0, 4, 5, 6, 7, 8, 9][..]),
        (3, &[0, 4, 5, 6, 7, 8, 9, 10, 11][..]),
    ] {
        let primary = serde_json::to_vec(&serde_json::json!({
            "asset": {"version": "2.0"}, "scene": 0,
            "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
            "buffers": vec![buffer.clone(); buffer_count],
            "bufferViews": [
                {"buffer": 0, "byteLength": 36},
                {"buffer": if buffer_count == 1 { 0 } else { 1 }, "byteOffset": 36, "byteLength": 2},
                {"buffer": if buffer_count == 1 { 0 } else { 2 }, "byteOffset": 40, "byteLength": 24},
            ],
            "accessors": [{
                "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                "min": [0, 0, 0], "max": [1, 1, 0],
                "sparse": {
                    "count": 2,
                    "indices": {"bufferView": 1, "componentType": 5121},
                    "values": {"bufferView": 2},
                },
            }],
        }))
        .unwrap();
        let observation = run(
            |ctx, physical, _| {
                let document: serde_json::Value =
                    serde_json::from_slice(ctx.services()?.primary_bytes()).unwrap();
                let declared_buffers = document["buffers"].as_array().unwrap().len();
                let expected = match declared_buffers {
                    1 => vec![
                        entity(SourceEntityKind::Primitive, Some(0), 0),
                        entity(SourceEntityKind::Buffer, None, 0),
                        entity(SourceEntityKind::BufferView, None, 0),
                        entity(SourceEntityKind::BufferView, None, 1),
                        entity(SourceEntityKind::BufferView, None, 2),
                        entity(SourceEntityKind::Accessor, None, 0),
                    ],
                    3 => vec![
                        entity(SourceEntityKind::Primitive, Some(0), 0),
                        entity(SourceEntityKind::Buffer, None, 0),
                        entity(SourceEntityKind::Buffer, None, 1),
                        entity(SourceEntityKind::Buffer, None, 2),
                        entity(SourceEntityKind::BufferView, None, 0),
                        entity(SourceEntityKind::BufferView, None, 1),
                        entity(SourceEntityKind::BufferView, None, 2),
                        entity(SourceEntityKind::Accessor, None, 0),
                    ],
                    _ => panic!("the sparse fixture has one or three Buffer declarations"),
                };
                let model =
                    super::super::read_normalized_gltf(ctx, &crate::GltfImportOptions::default())?;
                assert_eq!(model.buffers.external_resource_count(), 0);
                assert_eq!(model.geometry.logical_meshes.len(), 1);
                let primitives = model.geometry.logical_meshes.as_slice()[0]
                    .primitives
                    .as_slice();
                assert_eq!(primitives.len(), 1);
                assert_eq!(model.geometry.primitive_provenance.as_slice(), &[0]);
                let attributes = primitives[0].attributes.as_slice();
                assert_eq!(attributes.len(), 1);
                assert_eq!(attributes[0].provenance.as_slice(), &[0]);
                assert_eq!(
                    attributes[0].values.as_slice(),
                    &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
                );
                for owner in [ProfileOwner::Primitive(0), ProfileOwner::Attribute(0)] {
                    assert_eq!(owner_identities(ctx, &model.sources, owner)?, expected);
                }
                physical.store(expected.len(), AtomicOrdering::Relaxed);
                Ok(())
            },
            None,
            None,
            &primary,
        );
        assert_eq!(observation.physical, expected_sources.len() - 1);
        assert!(matches!(
            observation.failure.cause(),
            ImportCause::Format(_)
        ));

        let report = plan
            .read_bytes(
                AssetPath::new("sparse-owner.gltf").unwrap(),
                &primary,
                ImportOperationControl::default(),
            )
            .unwrap();
        let scene = report.scene();
        let mesh = scene.meshes().next().unwrap();
        let primitive = mesh.primitives().next().unwrap();
        let position = primitive.positions().unwrap();
        for owner in [primitive.metadata_owner(), position.metadata_owner()] {
            let translated = scene
                .owner_provenance(owner)
                .unwrap()
                .map(|node| {
                    let baozi_core::ProvenanceKindRef::Source(source) = node.kind() else {
                        panic!("sparse owners retain source leaves");
                    };
                    assert_eq!(source.resource().index(), 0);
                    source.index()
                })
                .collect::<Vec<_>>();
            assert_eq!(translated, expected_sources);
        }
    }
}

#[test]
fn actual_suspended_owners_keep_default_sampler_revisits_and_clip_input_only_declarations() {
    let geometry = [
        0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0,
    ]
    .into_iter()
    .flat_map(f32::to_le_bytes)
    .collect::<Vec<_>>();
    let input = [0.0_f32, 1.0, 0.0, 3.0]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>();
    let output = [1.0_f32, 2.0, 3.0, 4.0, 5.0, 6.0]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>();
    let primary = serde_json::to_vec(&serde_json::json!({
        "asset": {"version": "2.0"}, "scene": 0,
        "scenes": [{"nodes": [2]}, {"nodes": [3]}],
        "nodes": [{"mesh": 0}, {}, {"children": [0, 1]}, {}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "TEXCOORD_0": 1}, "material": 0}]}],
        "materials": [{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}, "metallicRoughnessTexture": {"index": 1}}, "emissiveTexture": {"index": 2}}],
        "images": [{"uri": "data:image/png;base64,AA=="}], "samplers": [{}],
        "textures": [{"source": 0}, {"source": 0, "sampler": 0}, {"source": 0}],
        "buffers": [
            {"byteLength": 60, "uri": format!("data:application/octet-stream;base64,{}", base64::engine::general_purpose::STANDARD.encode(geometry))},
            {"byteLength": 16, "uri": "input.bin"}, {"byteLength": 24, "uri": "output.bin"},
            {"byteLength": 32, "uri": "ignored.bin"}
        ],
        "bufferViews": [
            {"buffer": 0, "byteLength": 36}, {"buffer": 0, "byteOffset": 36, "byteLength": 24},
            {"buffer": 1, "byteLength": 8}, {"buffer": 2, "byteLength": 24},
            {"buffer": 1, "byteOffset": 8, "byteLength": 8},
            {"buffer": 3, "byteLength": 8}, {"buffer": 3, "byteOffset": 8, "byteLength": 24}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0]},
            {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC2"},
            {"bufferView": 2, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [1]},
            {"bufferView": 3, "componentType": 5126, "count": 2, "type": "VEC3"},
            {"bufferView": 4, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [3]},
            {"bufferView": 5, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [9]},
            {"bufferView": 6, "componentType": 5126, "count": 2, "type": "VEC3"}
        ],
        "animations": [
            {"samplers": [{"input": 4, "output": 3}], "channels": [{"sampler": 0, "target": {"node": 3, "path": "translation"}}]},
            {"samplers": [{"input": 2, "output": 3}, {"input": 4, "output": 3}, {"input": 5, "output": 6}], "channels": [
                {"sampler": 2, "target": {"path": "translation"}},
                {"sampler": 1, "target": {"node": 3, "path": "translation"}},
                {"sampler": 0, "target": {"node": 0, "path": "translation"}},
                {"sampler": 0, "target": {"node": 1, "path": "scale"}}
            ]},
            {"samplers": [{"input": 2, "output": 3}], "channels": [{"sampler": 0, "target": {"node": 0, "path": "translation"}}]}
        ]
    })).unwrap();
    let observation = run_with_assets(
        |ctx, physical, _| {
            let model =
                super::super::read_normalized_gltf(ctx, &crate::GltfImportOptions::default())?;
            let profile = &model.sources;
            for (owner, source_index) in [(0, 2), (1, 0), (2, 1)] {
                assert_eq!(
                    owner_identities(
                        ctx,
                        profile,
                        ProfileOwner::Node(baozi_import::authoring::NodeLocalId::new(owner))
                    )?,
                    [entity(SourceEntityKind::Node, None, source_index)]
                );
            }
            assert_eq!(
                owner_identities(ctx, profile, ProfileOwner::Sampler(0))?,
                [entity(SourceEntityKind::Sampler, None, 0)]
            );
            assert_eq!(
                owner_identities(ctx, profile, ProfileOwner::Sampler(1))?,
                [
                    entity(SourceEntityKind::Texture, None, 0),
                    entity(SourceEntityKind::Texture, None, 2)
                ]
            );
            assert_eq!(
                owner_identities(ctx, profile, ProfileOwner::AnimationClip(0))?,
                [
                    entity(SourceEntityKind::Buffer, None, 1),
                    entity(SourceEntityKind::BufferView, None, 2),
                    entity(SourceEntityKind::BufferView, None, 4),
                    entity(SourceEntityKind::Accessor, None, 2),
                    entity(SourceEntityKind::Accessor, None, 4),
                    entity(SourceEntityKind::Animation, None, 1),
                    entity(SourceEntityKind::AnimationSampler, Some(1), 0),
                    entity(SourceEntityKind::AnimationSampler, Some(1), 1),
                ]
            );
            assert_eq!(
                owner_identities(ctx, profile, ProfileOwner::AnimationClip(1))?,
                [
                    entity(SourceEntityKind::Buffer, None, 1),
                    entity(SourceEntityKind::BufferView, None, 2),
                    entity(SourceEntityKind::Accessor, None, 2),
                    entity(SourceEntityKind::Animation, None, 2),
                    entity(SourceEntityKind::AnimationSampler, Some(2), 0),
                ]
            );
            for (canonical, animation, channel) in [(0, 1, 2), (1, 1, 3), (2, 2, 0)] {
                assert_eq!(
                    owner_identities(ctx, profile, ProfileOwner::AnimationChannel(canonical))?,
                    [
                        entity(SourceEntityKind::Buffer, None, 1),
                        entity(SourceEntityKind::Buffer, None, 2),
                        entity(SourceEntityKind::BufferView, None, 2),
                        entity(SourceEntityKind::BufferView, None, 3),
                        entity(SourceEntityKind::Accessor, None, 2),
                        entity(SourceEntityKind::Accessor, None, 3),
                        entity(SourceEntityKind::AnimationChannel, Some(animation), channel),
                        entity(SourceEntityKind::AnimationSampler, Some(animation), 0),
                    ]
                );
            }
            assert_eq!(model.animations.clip_count(), 2);
            assert_eq!(model.animations.channel_count(), 3);
            assert_eq!(model.animations.clip_provenance(0), Some(&[1, 2][..]));
            assert_eq!(model.animations.clip_provenance(1), Some(&[1, 2][..]));
            for id in [
                entity(SourceEntityKind::Animation, None, 0),
                entity(SourceEntityKind::Buffer, None, 3),
                entity(SourceEntityKind::AnimationSampler, Some(1), 2),
            ] {
                assert!(profile.required(ctx, id).is_err());
            }
            physical.store(1, AtomicOrdering::Relaxed);
            Ok(())
        },
        ResourceLimits::default(),
        None,
        &primary,
        &[("input.bin", &input), ("output.bin", &output)],
    );
    assert_eq!(observation.physical, 1);
}

const DOCUMENT_EXTRAS: &str = r#"{ "标签" : [-0, 184467440737095516160, "\u0061"] }"#;
const SCENE_ROW: &str = r#"{"nodes":[0],"extras":[]}"#;
const NODE_ROW: &str = r#"{"name":"节点","camera":0,"extr\u0061s":false}"#;
const CAMERA_ROW: &str = r#"{"type":"perspective","extras":true,"perspective":{"yfov":1,"znear":0.1,"extras":{"nested":-0.00}}}"#;
const PROFILE_JSON: &str = r#"{"asset":{"version":"2.0","extras":null},"extras":{ "标签" : [-0, 184467440737095516160, "\u0061"] },"scene":0,"scenes":[{"nodes":[0],"extras":[]}],"nodes":[{"name":"节点","camera":0,"extr\u0061s":false},{"extras":"unselected"}],"cameras":[{"type":"perspective","extras":true,"perspective":{"yfov":1,"znear":0.1,"extras":{"nested":-0.00}}}]}"#;

const EXPECTED_EXTRAS: [(usize, &str, &str); 6] = [
    (0, "org.baozi.gltf:asset-extras", "null"),
    (0, "org.baozi.gltf:extras", DOCUMENT_EXTRAS),
    (1, "org.baozi.gltf:extras", "[]"),
    (2, "org.baozi.gltf:extras", "false"),
    (3, "org.baozi.gltf:extras", "true"),
    (
        3,
        "org.baozi.gltf:perspective-extras",
        r#"{"nested":-0.00}"#,
    ),
];

fn glb(json: &[u8]) -> Vec<u8> {
    let mut padded = json.to_vec();
    while !padded.len().is_multiple_of(4) {
        padded.push(b' ');
    }
    let mut output = Vec::new();
    output.extend_from_slice(b"glTF");
    output.extend_from_slice(&2_u32.to_le_bytes());
    output.extend_from_slice(&(20_u32 + padded.len() as u32).to_le_bytes());
    output.extend_from_slice(&(padded.len() as u32).to_le_bytes());
    output.extend_from_slice(b"JSON");
    output.extend_from_slice(&padded);
    output
}

fn json_base(bytes: &[u8]) -> usize {
    if bytes.starts_with(b"glTF") { 20 } else { 0 }
}

#[test]
fn absent_and_single_null_extras_skip_comparisons_without_losing_presence() {
    const ABSENT: &[u8] = br#"{"asset":{"version":"2.0"}}"#;
    const NULL: &[u8] = br#"{"asset":{"version":"2.0"},"extras":null}"#;
    for primary in [ABSENT, NULL] {
        let observation = run(
            |ctx, _, _| {
                let absent = ctx.services()?.primary_bytes() == ABSENT;
                let map = GltfSourceMap::from_primary_for_test(ctx)?;
                let selected = SelectedSourceProfile::from_source_map(ctx, &map, |_| true)?;
                if absent {
                    assert!(selected.extras().is_empty());
                } else {
                    assert_eq!(selected.extras().len(), 1);
                    assert_eq!(selected.extras()[0].source, DOCUMENT);
                    assert_eq!(selected.extras()[0].key.as_str(), "org.baozi.gltf:extras");
                    assert_eq!(selected.extras()[0].bytes(&mut ctx.services()?)?, b"null");
                }
                Ok(())
            },
            None,
            None,
            primary,
        );
        assert!(matches!(
            observation.failure.cause(),
            ImportCause::Format(_)
        ));
        assert_eq!(observation.comparisons(), 0);
    }
}

#[test]
fn actual_extras_sort_retains_literal_source_keys_ranges_bytes_and_resource_indices() {
    for primary in [
        PROFILE_JSON.as_bytes().to_vec(),
        glb(PROFILE_JSON.as_bytes()),
    ] {
        let observation = run(
            |ctx, _, _| {
                let base = json_base(ctx.services()?.primary_bytes());
                let map = GltfSourceMap::from_primary_for_test(ctx)?;
                let selected = SelectedSourceProfile::from_source_map(ctx, &map, |id| {
                    id != entity(SourceEntityKind::Node, None, 1)
                })?;
                let ids = [
                    DOCUMENT,
                    entity(SourceEntityKind::Scene, None, 0),
                    entity(SourceEntityKind::Node, None, 0),
                    entity(SourceEntityKind::Camera, None, 0),
                ];
                assert_eq!(
                    selected
                        .entries()
                        .iter()
                        .map(|source| source.entity.id)
                        .collect::<Vec<_>>(),
                    ids
                );
                for (index, raw) in [PROFILE_JSON, SCENE_ROW, NODE_ROW, CAMERA_ROW]
                    .into_iter()
                    .enumerate()
                {
                    let offset = base + PROFILE_JSON.find(raw).unwrap();
                    let source = &selected.entries()[index];
                    assert_eq!(source.entity.range.byte_offset, offset as u64);
                    assert_eq!(source.entity.range.byte_length, raw.len() as u64);
                    let kind = ["document", "scene", "node", "camera"][index];
                    assert_eq!(
                        source.locator.namespaced_value().unwrap().as_str(),
                        format!(
                            "kind={kind};parent=-;index=0;offset={offset};length={}",
                            raw.len()
                        )
                    );
                    assert_eq!(
                        selected.source_index(ctx, 3, ids[index])?,
                        if index == 0 { 0 } else { index as u32 + 2 }
                    );
                }
                assert_eq!(selected.extras().len(), 6);
                for (extra, (index, key, raw)) in selected.extras().iter().zip(EXPECTED_EXTRAS) {
                    assert_eq!(extra.source, ids[index]);
                    assert_eq!(extra.key.as_str(), key);
                    assert_eq!(
                        extra.range.byte_offset,
                        (base + PROFILE_JSON.find(raw).unwrap()) as u64
                    );
                    assert_eq!(extra.range.byte_length, raw.len() as u64);
                    assert_eq!(extra.bytes(&mut ctx.services()?)?, raw.as_bytes());
                }
                Ok(())
            },
            None,
            None,
            &primary,
        );
        assert!(matches!(
            observation.failure.cause(),
            ImportCause::Format(_)
        ));
        assert!(observation.completed[ComparisonSite::ExtrasSort as usize] > 0);
    }
}

#[test]
fn extras_sort_cancellation_is_observed_before_profile_success() {
    let observation = run(
        |ctx, _, _| {
            let map = GltfSourceMap::from_primary_for_test(ctx)?;
            SelectedSourceProfile::from_source_map(ctx, &map, |_| true)?;
            panic!("the short extras sort must stop at its scope-exit poll");
        },
        None,
        Some((ComparisonSite::ExtrasSort, 1)),
        PROFILE_JSON.as_bytes(),
    );
    assert_cancelled(&observation.failure);
    assert!((1..64).contains(&observation.completed[ComparisonSite::ExtrasSort as usize]));
}

#[test]
fn public_import_preserves_literal_profiles_and_extras_after_report_drop_for_json_and_glb() {
    let mut registry = RegistryBuilder::new();
    registry.register(crate::GltfImporter).unwrap();
    let plan = registry
        .freeze(RegistryBuildLimits::default())
        .unwrap()
        .plan(&ImportRequest::new(), OptionCompileLimits::default())
        .unwrap();
    for primary in [
        PROFILE_JSON.as_bytes().to_vec(),
        glb(PROFILE_JSON.as_bytes()),
    ] {
        let base = json_base(&primary);
        let report = plan
            .read_bytes(
                AssetPath::new("literal-profile.gltf").unwrap(),
                &primary,
                ImportOperationControl::default(),
            )
            .unwrap();
        assert!(report.evidence().matches_scene(report.scene()));
        assert!(report.loss_evidence().is_empty());
        let scene = report.scene();
        let snapshot = scene
            .capture_snapshot(SceneSnapshotLimits::try_new(32 * 1024, 128, 1_000_000).unwrap())
            .unwrap();
        assert_eq!(snapshot.profile_resource_count(), 1);
        assert_eq!(snapshot.profile_source_count(), 4);
        assert_eq!(snapshot.profile_provenance_node_count(), 5);
        assert_eq!(snapshot.profile_operation_count(), 1);
        assert_eq!(snapshot.profile_evidence_count(), 1);
        assert_eq!(snapshot.profile_parent_reference_count(), 2);
        assert_eq!(snapshot.profile_metadata_entry_count(), 6);
        let sources = scene.source_identities().collect::<Vec<_>>();
        for (source, (kind, raw)) in sources.iter().zip([
            ("document", PROFILE_JSON),
            ("scene", SCENE_ROW),
            ("node", NODE_ROW),
            ("camera", CAMERA_ROW),
        ]) {
            let SourceObjectLocatorRef::Namespaced { value, .. } = source.locator() else {
                panic!("selected glTF source schema");
            };
            let offset = base + PROFILE_JSON.find(raw).unwrap();
            assert_eq!(
                value,
                format!(
                    "kind={kind};parent=-;index=0;offset={offset};length={}",
                    raw.len()
                )
            );
        }
        let detached = scene
            .metadata_entries()
            .map(|entry| {
                let MetadataOwnerRef::SourceObject(source) = entry.owner() else {
                    panic!("source extras have source-object owners");
                };
                let MetadataValueRef::Bytes {
                    media_type,
                    payload,
                } = entry.value()
                else {
                    panic!("source extras retain raw JSON bytes");
                };
                assert_eq!(media_type, "application/json");
                (source.index(), entry.key().to_owned(), payload.to_vec())
            })
            .collect::<Vec<_>>();
        drop(sources);
        drop(report);
        assert_eq!(detached.len(), EXPECTED_EXTRAS.len());
        for ((owner, key, bytes), (expected_owner, expected_key, raw)) in
            detached.iter().zip(EXPECTED_EXTRAS)
        {
            assert_eq!(*owner, expected_owner);
            assert_eq!(key, expected_key);
            assert_eq!(bytes, raw.as_bytes());
        }
    }
}
