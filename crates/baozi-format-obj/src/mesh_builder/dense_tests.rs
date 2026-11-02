use super::{BuiltGeometryStreams, CornerChannelSelection, dense};
use baozi_import::{
    ImportCause, ImportConfigOverlay, ImportDimension, ImportFailure, ImportOperationControl,
    ImportReport, ImportRequest, ImportRequestLayer, ImportTerminalFailureClass,
    OptionCompileLimits, ParseExecution, ParseResult, RegistryBuildLimits, RegistryBuilder,
    ResourceLimits, ResourceLimitsOverlay, ScopeAbortCause,
};
use baozi_io::AssetPath;
use baozi_operation::{CancellationToken, ControlCause};
use std::{cell::RefCell, marker::PhantomData, rc::Rc};

const PASSES: [&str; 11] = [
    "obj-dense-position-init-work",
    "obj-dense-position-discovery-work",
    "obj-dense-texcoord-init-work",
    "obj-dense-texcoord-discovery-work",
    "obj-dense-normal-init-work",
    "obj-dense-normal-discovery-work",
    "obj-dense-texcoord-projection-work",
    "obj-dense-texcoord-indices-work",
    "obj-dense-normal-projection-work",
    "obj-dense-normal-indices-work",
    "obj-dense-position-values-work",
];

#[derive(Debug)]
struct Captured {
    position_sources: Vec<usize>,
    texcoord_sources: Option<Vec<usize>>,
    normal_sources: Option<Vec<usize>>,
    positions: Vec<u32>,
    indices: Vec<u32>,
    texcoord_lanes: u8,
    texcoords: Option<Vec<u32>>,
    texcoord_indices: Option<Vec<u32>>,
    normals: Option<Vec<u32>>,
    normal_indices: Option<Vec<u32>>,
}

struct Probe {
    target: &'static str,
    cancel_after: Option<usize>,
    token: Option<CancellationToken>,
    started: bool,
    completed: usize,
    routes: Vec<bool>,
    captured: Vec<Captured>,
}

thread_local! {
    static PROBE: RefCell<Option<Probe>> = const { RefCell::new(None) };
}

struct ProbeGuard(PhantomData<Rc<()>>);

impl ProbeGuard {
    fn install(
        target: &'static str,
        cancel_after: Option<usize>,
        token: CancellationToken,
    ) -> Self {
        PROBE.with(|slot| {
            let mut slot = slot.borrow_mut();
            assert!(slot.is_none());
            *slot = Some(Probe {
                target,
                cancel_after,
                token: Some(token),
                started: false,
                completed: 0,
                routes: Vec::new(),
                captured: Vec::new(),
            });
        });
        Self(PhantomData)
    }

    fn finish(self) -> Probe {
        PROBE.with(|slot| slot.borrow_mut().take().expect("active dense OBJ probe"))
    }
}

impl Drop for ProbeGuard {
    fn drop(&mut self) {
        let probe = PROBE.with(|slot| slot.borrow_mut().take());
        drop(probe);
    }
}

pub(super) fn before_rows(
    exec: &mut ParseExecution<'_, '_>,
    resource: &'static str,
) -> ParseResult<()> {
    let token = PROBE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let probe = slot.as_mut()?;
        if probe.target != resource {
            return None;
        }
        probe.started = true;
        if probe.cancel_after == Some(0) {
            probe.token.take()
        } else {
            None
        }
    });
    if let Some(token) = token {
        token.cancel();
        exec.poll()?;
    }
    Ok(())
}

pub(super) fn row_completed(resource: &'static str) {
    let token = PROBE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let probe = slot.as_mut()?;
        if probe.target != resource {
            return None;
        }
        probe.completed += 1;
        if probe.cancel_after == Some(probe.completed) {
            probe.token.take()
        } else {
            None
        }
    });
    if let Some(token) = token {
        token.cancel();
    }
}

pub(super) fn general_segment() {
    PROBE.with(|slot| {
        if let Some(probe) = slot.borrow_mut().as_mut() {
            probe.routes.push(false);
        }
    });
}

pub(super) fn capture(
    positions: &[usize],
    texcoords: Option<&[usize]>,
    normals: Option<&[usize]>,
    built: &BuiltGeometryStreams<'_>,
) {
    PROBE.with(|slot| {
        if let Some(probe) = slot.borrow_mut().as_mut() {
            probe.routes.push(true);
            probe.captured.push(Captured {
                position_sources: positions.to_vec(),
                texcoord_sources: texcoords.map(<[usize]>::to_vec),
                normal_sources: normals.map(<[usize]>::to_vec),
                positions: bits(built.positions.as_slice()),
                indices: built.indices.as_slice().to_vec(),
                texcoord_lanes: built.texcoord_lanes,
                texcoords: built
                    .texcoords
                    .as_ref()
                    .map(|p| bits(p.source_values.as_slice())),
                texcoord_indices: built
                    .texcoords
                    .as_ref()
                    .map(|p| p.source_indices.as_slice().to_vec()),
                normals: built
                    .normals
                    .as_ref()
                    .map(|p| bits(p.source_values.as_slice().as_flattened())),
                normal_indices: built
                    .normals
                    .as_ref()
                    .map(|p| p.source_indices.as_slice().to_vec()),
            });
        }
    });
}

fn bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|value| value.to_bits()).collect()
}

#[allow(
    clippy::result_large_err,
    reason = "the test inspects the complete terminal failure evidence"
)]
fn read(
    bytes: &[u8],
    max_work: Option<u64>,
    token: CancellationToken,
) -> Result<ImportReport, ImportFailure> {
    let mut registry = RegistryBuilder::new();
    crate::register(&mut registry).unwrap();
    let mut limits = ResourceLimits::default();
    if let Some(limit) = max_work {
        limits.max_work_units = limit;
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
    plan.read_bytes(
        AssetPath::new("dense-remap.obj").unwrap(),
        bytes,
        ImportOperationControl::default().with_cancellation(token),
    )
    .map_err(|failure| {
        failure
            .into_operation_failure()
            .expect("ordinary OBJ operation failure")
    })
}

fn failed_pass(
    bytes: &[u8],
    target: &'static str,
    cancel_after: Option<usize>,
    max_work: Option<u64>,
) -> (ImportFailure, Probe) {
    let token = CancellationToken::new();
    let guard = ProbeGuard::install(target, cancel_after, token.clone());
    let failure = read(bytes, max_work, token).expect_err("selected dense pass must fail");
    let probe = guard.finish();
    assert!(probe.started, "did not reach {target}");
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
    (failure, probe)
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

fn wide_document(count: usize) -> String {
    let mut document = String::new();
    for source in 0..count {
        document.push_str(&format!("v {source} 0 0\nvt {source} 0\nvn 0 0 1\n"));
    }
    document.push('f');
    for source in (1..=count).rev() {
        document.push_str(&format!(" {source}/{source}/{source}"));
    }
    document.push('\n');
    document
}

#[test]
fn dense_choice_accounts_for_domains_minimum_capacity_and_overflow() {
    let both = CornerChannelSelection {
        texcoords: true,
        normals: true,
    };
    let neither = CornerChannelSelection {
        texcoords: false,
        normals: false,
    };
    assert!(!dense::eligible(9, [10, 8, 8], both));
    assert!(!dense::eligible(9, [8, 10, 8], both));
    assert!(!dense::eligible(9, [8, 8, 10], both));
    assert!(!dense::eligible(1, [1, 1, 1], both));
    assert!(dense::eligible(9, [8, 8, 8], both));
    assert!(dense::eligible(6, [3, usize::MAX, usize::MAX], neither));
    assert!(!dense::eligible(usize::MAX, [usize::MAX; 3], both));
    // This allocation-free decision must not impose an unrelated fixed byte ceiling.
    assert!(dense::eligible(1 << 20, [1 << 19; 3], both));
}

fn identity_document() -> String {
    let mut document = String::new();
    for source in 0..8 {
        let x = if source == 5 { 2 } else { source };
        if source == 7 {
            document.push_str("v -0 7 -0\nvt -0 0.75\nvn -0 1 -0\n");
        } else if source == 0 {
            document.push_str("v 0 0 0\nvt 99 99 99\nvn 0 0 1\n");
        } else {
            document.push_str(&format!("v {x} 0 0\nvt 0.5 -0\nvn 0 0 1\n"));
        }
    }
    document.push_str("g first\nf 8/8/8 3/3/3 8/8/8 6/6/6 3/3/3 6/6/6 8/8/8 3/3/3 6/6/6\n");
    document
}

#[test]
fn dense_staging_preserves_source_identity_order_and_signed_zero() {
    let token = CancellationToken::new();
    let guard = ProbeGuard::install("", None, token.clone());
    let report = read(identity_document().as_bytes(), None, token).unwrap();
    let probe = guard.finish();
    assert_eq!(probe.routes, [true]);
    let built = &probe.captured[0];
    assert_eq!(built.position_sources, [7, 2, 5]);
    assert_eq!(built.texcoord_sources.as_deref(), Some(&[2, 5, 7][..]));
    assert_eq!(built.normal_sources.as_deref(), Some(&[2, 5, 7][..]));
    assert_eq!(built.indices, [0, 1, 0, 2, 1, 2, 0, 1, 2]);
    assert_eq!(
        built.positions,
        bits(&[-0.0, 7.0, -0.0, 2.0, 0.0, 0.0, 2.0, 0.0, 0.0]),
    );
    assert_eq!(built.texcoord_lanes, 2);
    assert_eq!(
        built.texcoords.as_deref(),
        Some(bits(&[0.5, -0.0, 0.5, -0.0, -0.0, 0.75]).as_slice()),
    );
    assert_eq!(
        built.normals.as_deref(),
        Some(bits(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, -0.0, 1.0, -0.0]).as_slice()),
    );
    for indices in [&built.texcoord_indices, &built.normal_indices] {
        assert_eq!(indices.as_deref(), Some(&[2, 0, 2, 1, 0, 1, 2, 0, 1][..]));
    }
    let primitive = report
        .scene()
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap();
    assert_eq!(primitive.vertex_count(), 3);
    assert_eq!(primitive.indices(), Some(built.indices.as_slice()));
    let (scene, evidence) = report.into_parts();
    assert!(evidence.matches_scene(&scene));
}

#[test]
fn dense_and_sparse_segments_choose_independently_and_do_not_alias() {
    let mut document = identity_document();
    document.push_str("g sparse\nf 8/8/8 3/3/3 6/6/6\n");
    document.push_str("g last\nf 3/3/3 8/8/8 6/6/6 3/3/3 6/6/6 8/8/8 3/3/3 6/6/6 8/8/8\n");
    let token = CancellationToken::new();
    let guard = ProbeGuard::install("", None, token.clone());
    let report = read(document.as_bytes(), None, token).unwrap();
    let probe = guard.finish();
    assert_eq!(probe.routes, [true, false, true]);
    assert_eq!(probe.captured[0].position_sources, [7, 2, 5]);
    assert_eq!(probe.captured[1].position_sources, [2, 7, 5]);
    assert_eq!(probe.captured[1].indices, [0, 1, 2, 0, 2, 1, 0, 2, 1]);
    assert_eq!(report.scene().mesh_count(), 3);
    let vertices = report
        .ledger()
        .dimension(ImportDimension::Vertices)
        .unwrap();
    assert_eq!((vertices.attempted(), vertices.consumed()), (17, 17));
}

#[test]
fn every_dense_pass_cancels_at_a_physical_prefix_and_preserves_short_tails() {
    let document = wide_document(130);
    for target in PASSES {
        let (before, probe) = failed_pass(document.as_bytes(), target, Some(0), None);
        assert_cancelled(&before);
        assert_eq!(probe.completed, 0);
        let (attempted, consumed) = counts(&before);
        assert_eq!(attempted, consumed);
        for (cancel_after, expected) in [(3, 32), (129, 130)] {
            let (failure, probe) =
                failed_pass(document.as_bytes(), target, Some(cancel_after), None);
            assert_cancelled(&failure);
            assert_eq!(probe.completed, expected, "{target}");
            assert!(probe.completed - cancel_after <= 31, "{target}");
            let prefix = consumed + expected as u64;
            assert_eq!(counts(&failure), (prefix, prefix), "{target}");
        }
    }
}

#[test]
fn denied_dense_pass_admission_never_mutates_or_consumes_the_next_rows() {
    let document = wide_document(130);
    for target in PASSES {
        let (before, _) = failed_pass(document.as_bytes(), target, Some(0), None);
        let (_, consumed) = counts(&before);
        let (failure, probe) = failed_pass(document.as_bytes(), target, None, Some(consumed));
        match failure.cause() {
            ImportCause::Scope(abort) => {
                assert!(matches!(abort.cause(), ScopeAbortCause::Ledger(_)));
            }
            ImportCause::OperationTerminal(terminal) => {
                assert_eq!(terminal.class(), ImportTerminalFailureClass::Ledger);
            }
            cause => panic!("expected dense admission rejection, got {cause:?}"),
        }
        assert_eq!(probe.completed, 0, "{target}");
        assert_eq!(counts(&failure), (consumed + 32, consumed), "{target}");
    }
}

#[test]
fn singleton_sources_do_not_alias_zero_or_consume_a_full_work_chunk() {
    let document = b"v -0 0 -0\nvt -0 0 0\nvn -0 0 1\nf 1/1/1 1/1/1 1/1/1 1/1/1 1/1/1 1/1/1 1/1/1 1/1/1 1/1/1\n";
    let token = CancellationToken::new();
    let guard = ProbeGuard::install("", None, token.clone());
    let report = read(document, None, token).unwrap();
    let probe = guard.finish();
    let built = &probe.captured[0];
    assert_eq!(built.position_sources, [0]);
    assert_eq!(built.texcoord_sources.as_deref(), Some(&[0][..]));
    assert_eq!(built.normal_sources.as_deref(), Some(&[0][..]));
    assert_eq!(built.indices, [0; 9]);
    assert_eq!(built.positions, bits(&[-0.0, 0.0, -0.0]));
    assert_eq!(built.texcoord_lanes, 3);
    let primitive = report
        .scene()
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap();
    assert_eq!(primitive.vertex_count(), 1);
    for target in PASSES {
        let (before, _) = failed_pass(document, target, Some(0), None);
        let (_, consumed) = counts(&before);
        let (failure, probe) = failed_pass(document, target, Some(1), None);
        assert_cancelled(&failure);
        let expected = if target.contains("discovery") || target.contains("indices") {
            9
        } else {
            1
        };
        assert_eq!(probe.completed, expected, "{target}");
        assert_eq!(
            counts(&failure),
            (consumed + expected as u64, consumed + expected as u64),
            "{target}"
        );
    }
}
