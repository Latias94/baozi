use super::*;
use baozi_import::{
    CompiledOptions, DetectionOutcome, DetectionResult, DetectionScope, Format, FormatDescriptor,
    ImportCause, ImportConfigOverlay, ImportDimension, ImportFailure, ImportOperationControl,
    ImportRequest, ImportRequestLayer, ImportTerminalFailureClass, MetadataOwnerRef,
    MetadataValueRef, NoFormatOptions, OptionCompileError, OptionCompileLimits, ParseContext,
    Probe, ReadConfidence, ReadHint, RegistryBuildLimits, RegistryBuildScope, RegistryBuilder,
    ResourceLimits, ResourceLimitsOverlay, SceneSnapshotLimits, ScopeAbortCause,
    SourceObjectLocatorRef,
};
use baozi_io::AssetPath;
use baozi_operation::{CancellationToken, ControlCause};
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
    LinkSort,
    LinkDedup,
}

struct CompletionProbe {
    site: ComparisonSite,
    completed: [usize; 4],
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
                completed: [0; 4],
                cancel_on,
            });
        });
        Self(PhantomData)
    }

    fn finish(self) -> [usize; 4] {
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
    let token = COMPLETION_PROBE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let probe = slot.as_mut()?;
        probe.completed[probe.site as usize] += 1;
        let (site, count, _) = probe.cancel_on.as_ref()?;
        if *site == probe.site && *count == probe.completed[probe.site as usize] {
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
    &mut ParseExecution<'region, 'session>,
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
        _: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome> {
        Ok(DetectionOutcome::Match(ReadConfidence::Certain))
    }

    fn parse(&self, _: &Self::Options, context: &mut ParseContext<'_>) -> ParseResult<()> {
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
    completed: [usize; 4],
}

impl Observation {
    fn comparisons(&self) -> u64 {
        self.completed.iter().sum::<usize>() as u64
    }
}

fn run(
    action: ComparisonAction,
    max_work: Option<u64>,
    cancel_on: Option<(ComparisonSite, usize)>,
    primary: &[u8],
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
    let mut limits = ResourceLimits::default();
    if let Some(max_work) = max_work {
        limits.max_work_units = max_work;
    }
    let request = ImportRequest::new().with_caller_layer(ImportRequestLayer::new(
        ImportConfigOverlay::default()
            .with_resource_limits(ResourceLimitsOverlay::from_limits(limits)),
    ));
    let plan = registry
        .freeze(RegistryBuildLimits::default())
        .unwrap()
        .plan(&request, OptionCompileLimits::default())
        .unwrap();
    let failure = plan
        .read_bytes(
            AssetPath::new("profile.gltf").unwrap(),
            primary,
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

fn profile<'session>(
    ctx: &mut ParseExecution<'_, 'session>,
    pairs: &[(ProfileOwner, SourceEntityId)],
) -> ParseResult<SelectedSourceProfile<'session>> {
    let mut profile = SelectedSourceProfile {
        entries: ctx.metered_vec(
            ImportStorageClass::FormatNormalization,
            "gltf-selected-source-identities",
        ),
        links: ctx.metered_vec(
            ImportStorageClass::FormatNormalization,
            "gltf-selected-source-owner-links",
        ),
        extras: ctx.metered_vec(
            ImportStorageClass::FormatNormalization,
            "gltf-selected-source-extras",
        ),
    };
    for &(owner, source) in pairs {
        profile
            .links
            .try_push(ctx, OwnerSourceLink { owner, source })?;
    }
    Ok(profile)
}

fn pairs(links: &[OwnerSourceLink]) -> Vec<(ProfileOwner, SourceEntityId)> {
    links.iter().map(|link| (link.owner, link.source)).collect()
}

fn numeric_comparisons(
    ctx: &mut ParseExecution<'_, '_>,
    physical: &AtomicUsize,
    _: &CancellationToken,
) -> Result<()> {
    let _storage = profile(ctx, &[(ProfileOwner::Scene, DOCUMENT); 16])?;
    let group = ctx.bind_work_group([ImportWorkClass::WorkUnits])?;
    ctx.with_work(&group, [0], |ctx, ledger_work| {
        let mut work = ProfileComparisonWork::new(ledger_work);
        for value in 0..8 {
            work.run(ctx, || {
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
    let actual_sort = run(finish_long_links, Some(before + 3), None, b"{}");
    assert_ledger_denial(&actual_sort.failure);
    assert_eq!(actual_sort.completed[ComparisonSite::LinkSort as usize], 3);
    assert_eq!(actual_sort.completed[ComparisonSite::LinkDedup as usize], 0);
    assert_eq!(counts(&actual_sort.failure), (before + 4, before + 3));
}

#[test]
fn known_error_closes_idle_scope_and_unwind_does_not_complete_the_interrupted_comparison() {
    let format_before = baseline();
    let ordinary = run(
        |ctx, physical, _| {
            let _storage = profile(ctx, &[(ProfileOwner::Scene, DOCUMENT); 16])?;
            let group = ctx.bind_work_group([ImportWorkClass::WorkUnits])?;
            ctx.with_work(&group, [0], |ctx, ledger_work| {
                let mut work = ProfileComparisonWork::new(ledger_work);
                for _ in 0..3 {
                    work.run(ctx, || physical.fetch_add(1, AtomicOrdering::Relaxed))?;
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
            let _storage = profile(ctx, &[(ProfileOwner::Scene, DOCUMENT); 16])?;
            let group = ctx.bind_work_group([ImportWorkClass::WorkUnits])?;
            ctx.with_work(&group, [0], |ctx, ledger_work| {
                let mut work = ProfileComparisonWork::new(ledger_work);
                for _ in 0..3 {
                    work.run(ctx, || physical.fetch_add(1, AtomicOrdering::Relaxed))?;
                }
                work.run::<()>(ctx, || {
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
fn link_order_dedup_capacity_and_comparison_counts_match_the_original_metered_algorithms() {
    let observation = run(
        |ctx, physical, _| {
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
            let input = [
                expected[7],
                expected[6],
                expected[5],
                expected[4],
                expected[3],
                expected[2],
                expected[1],
                expected[0],
                expected[5],
                expected[2],
                expected[7],
            ];
            let mut selected = profile(ctx, &input)?;
            let capacity = selected.links.capacity();
            selected.finish(ctx)?;
            assert_eq!(pairs(selected.links.as_slice()), expected);
            assert_eq!(selected.links.capacity(), capacity);

            // Use the existing container as the control, not a second sorting implementation.
            let mut control = profile(ctx, &input)?;
            let mut sort_calls = 0;
            control
                .links
                .try_sort_unstable_by(|left, right| -> Result<_> {
                    sort_calls += 1;
                    physical.fetch_add(1, AtomicOrdering::Relaxed);
                    Ok(left.cmp(right))
                })?;
            let mut dedup_calls = 0;
            let retained = control
                .links
                .try_partition_dedup_by(|left, right| -> Result<_> {
                    dedup_calls += 1;
                    physical.fetch_add(1, AtomicOrdering::Relaxed);
                    Ok(left == right)
                })?;
            control.links.truncate(retained);
            assert_eq!(pairs(control.links.as_slice()), expected);
            COMPLETION_PROBE.with(|slot| {
                let slot = slot.borrow();
                let completed = slot.as_ref().unwrap().completed;
                assert_eq!(completed[ComparisonSite::LinkSort as usize], sort_calls);
                assert_eq!(completed[ComparisonSite::LinkDedup as usize], dedup_calls);
            });
            assert_eq!(
                pairs(selected.for_owner(ctx, ProfileOwner::Attribute(0))?),
                expected[2..7]
            );
            assert_eq!(
                pairs(selected.for_owner(ctx, ProfileOwner::Scene)?),
                expected[..1]
            );
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
    assert_eq!(observation.comparisons(), observation.physical as u64);
    // Two unchanged length-eight owner lookups charge ten units each; truncation charges none.
    let expected = baseline() + observation.comparisons() + 20;
    assert_eq!(counts(&observation.failure), (expected, expected));
}

fn finish_long_links(
    ctx: &mut ParseExecution<'_, '_>,
    _: &AtomicUsize,
    _: &CancellationToken,
) -> Result<()> {
    let mut selected = profile(ctx, &[(ProfileOwner::Scene, DOCUMENT); 130])?;
    let capacity = selected.links.capacity();
    let result = selected.finish(ctx);
    assert!(result.is_err());
    assert_eq!(
        selected.links.len(),
        130,
        "cancelled dedup must not truncate its suffix"
    );
    assert_eq!(selected.links.capacity(), capacity);
    result
}

fn finish_short_links(
    ctx: &mut ParseExecution<'_, '_>,
    _: &AtomicUsize,
    _: &CancellationToken,
) -> Result<()> {
    let mut selected = profile(ctx, &[(ProfileOwner::Scene, DOCUMENT); 2])?;
    let capacity = selected.links.capacity();
    let result = selected.finish(ctx);
    assert!(result.is_err());
    assert_eq!(
        selected.links.len(),
        2,
        "scope-exit cancellation precedes truncation"
    );
    assert_eq!(selected.links.capacity(), capacity);
    result
}

#[test]
fn actual_sort_and_dedup_cancel_within_64_comparisons_and_at_short_scope_exits() {
    let before = non_format_baseline();
    for (action, at_poll) in [
        (finish_long_links as ComparisonAction, 64),
        (finish_short_links as ComparisonAction, 1),
    ] {
        for site in [ComparisonSite::LinkSort, ComparisonSite::LinkDedup] {
            let observation = run(action, None, Some((site, 1)), b"{}");
            assert_cancelled(&observation.failure);
            assert_eq!(observation.completed[site as usize], at_poll);
            if site == ComparisonSite::LinkSort {
                assert_eq!(observation.completed[ComparisonSite::LinkDedup as usize], 0);
            }
            let expected = before + observation.comparisons();
            assert_eq!(counts(&observation.failure), (expected, expected));
        }
    }
}

#[test]
fn empty_singleton_finish_has_no_comparison_work_and_still_observes_entry_cancellation() {
    let before = baseline();
    let observation = run(
        |ctx, _, _| {
            for input in [&[][..], &[(ProfileOwner::Scene, DOCUMENT)][..]] {
                let mut selected = profile(ctx, input)?;
                let capacity = selected.links.capacity();
                selected.finish(ctx)?;
                assert_eq!(pairs(selected.links.as_slice()), input);
                assert_eq!(selected.links.capacity(), capacity);
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
        ctx: &mut ParseExecution<'_, '_>,
        _: &AtomicUsize,
        token: &CancellationToken,
    ) -> Result<()> {
        let mut selected = profile(ctx, &[])?;
        token.cancel();
        selected.finish(ctx)
    }
    fn cancel_singleton(
        ctx: &mut ParseExecution<'_, '_>,
        _: &AtomicUsize,
        token: &CancellationToken,
    ) -> Result<()> {
        let mut selected = profile(ctx, &[(ProfileOwner::Scene, DOCUMENT)])?;
        token.cancel();
        selected.finish(ctx)
    }
    let before = non_format_baseline();
    for action in [cancel_empty as ComparisonAction, cancel_singleton] {
        let observation = run(action, None, None, b"{}");
        assert_cancelled(&observation.failure);
        assert_eq!(observation.comparisons(), 0);
        assert_eq!(counts(&observation.failure), (before, before));
    }
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
                let absent = ctx.primary_bytes() == ABSENT;
                let map = GltfSourceMap::from_primary_for_test(ctx)?;
                let selected = SelectedSourceProfile::from_source_map(ctx, &map, |_| true)?;
                if absent {
                    assert!(selected.extras().is_empty());
                } else {
                    assert_eq!(selected.extras().len(), 1);
                    assert_eq!(selected.extras()[0].source, DOCUMENT);
                    assert_eq!(selected.extras()[0].key.as_str(), "org.baozi.gltf:extras");
                    assert_eq!(selected.extras()[0].bytes(ctx)?, b"null");
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
                let base = json_base(ctx.primary_bytes());
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
                    assert_eq!(extra.bytes(ctx)?, raw.as_bytes());
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
