use crate::FORMAT_ID;
use baozi_core::{AttributeData, TopologyKind, ValidatedScene};
use baozi_import::{
    ImportCause, ImportDimension, ImportOperationControl, ImportPhase, ImportRequest,
    OptionCompileLimits, RegistryBuildLimits, RegistryBuilder, ScopeAbortCause,
};
use baozi_io::AssetPath;
use baozi_operation::{CancellationOrigin, CancellationToken, ControlCause};
use std::{cell::RefCell, marker::PhantomData, rc::Rc};

const FACES: usize = 1_025;
const CANCEL_AFTER: usize = 257;

struct Probe {
    completed: usize,
    raised_at: Option<usize>,
    token: Option<CancellationToken>,
}

thread_local! {
    static PROBE: RefCell<Option<Probe>> = const { RefCell::new(None) };
}

struct ProbeGuard(PhantomData<Rc<()>>);

impl ProbeGuard {
    fn install(token: CancellationToken) -> Self {
        PROBE.with(|slot| {
            let mut slot = slot.borrow_mut();
            assert!(slot.is_none());
            *slot = Some(Probe {
                completed: 0,
                raised_at: None,
                token: Some(token),
            });
        });
        Self(PhantomData)
    }

    fn finish(self) -> Probe {
        PROBE.with(|slot| {
            slot.borrow_mut()
                .take()
                .expect("active OBJ completion probe")
        })
    }
}

impl Drop for ProbeGuard {
    fn drop(&mut self) {
        let probe = PROBE.with(|slot| slot.borrow_mut().take());
        drop(probe);
    }
}

pub(crate) fn face_completed(count: usize) {
    let token = PROBE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let probe = slot.as_mut()?;
        probe.completed = count;
        if count == CANCEL_AFTER && probe.raised_at.is_none() {
            probe.raised_at = Some(count);
            probe.token.take()
        } else {
            None
        }
    });
    if let Some(token) = token {
        token.cancel();
    }
}

fn assert_scene(scene: &ValidatedScene) {
    assert_eq!(scene.mesh_count(), 1);
    assert_eq!(scene.primitive_count(), 1);
    let mesh = scene.meshes().next().unwrap();
    let primitive = mesh.primitives().next().unwrap();
    assert_eq!(primitive.topology(), TopologyKind::TriangleList);
    assert_eq!(primitive.face_count() as usize, FACES);
    assert_eq!(primitive.corner_count() as usize, 3 * FACES);
    assert_eq!(primitive.vertex_count(), 3);
    let indices = primitive.indices().unwrap();
    assert_eq!(indices.len(), 3 * FACES);
    for face in indices.chunks_exact(3) {
        assert_eq!(face, [0, 1, 2]);
    }
    let positions = primitive.positions().unwrap();
    let AttributeData::DenseF32 { lanes: 3, values } = positions.data() else {
        panic!("OBJ must retain dense XYZ positions");
    };
    assert_eq!(values, [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
}

#[test]
fn cancellation_after_a_real_obj_face_preserves_the_exact_prefix() {
    let mut bytes = b"v 0 0 0\nv 1 0 0\nv 0 1 0\n".to_vec();
    for _ in 0..FACES {
        bytes.extend_from_slice(b"f 1 2 3\n");
    }
    assert!(bytes.len() > 8192);
    let source = AssetPath::new("control.obj").unwrap();
    let mut registry = RegistryBuilder::new();
    crate::register(&mut registry).unwrap();
    let plan = registry
        .freeze(RegistryBuildLimits::default())
        .unwrap()
        .plan(&ImportRequest::new(), OptionCompileLimits::default())
        .unwrap();
    let baseline = plan
        .read_bytes(source.clone(), &bytes, ImportOperationControl::default())
        .unwrap();
    let (retained, evidence) = baseline.into_parts();
    assert!(evidence.matches_scene(&retained));
    assert_scene(&retained);

    let token = CancellationToken::new();
    let probe = ProbeGuard::install(token.clone());
    let failure = plan
        .read_bytes(
            source.clone(),
            &bytes,
            ImportOperationControl::default().with_cancellation(token),
        )
        .expect_err("a completed face raises cancellation during actual OBJ parsing")
        .into_operation_failure()
        .expect("ordinary operation failure");
    let probe = probe.finish();
    assert_eq!(probe.raised_at, Some(CANCEL_AFTER));
    assert_eq!(probe.completed, CANCEL_AFTER);
    assert!(probe.completed < FACES);
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort) if matches!(
            abort.cause(),
            ScopeAbortCause::Control(ControlCause::Cancelled { origin: CancellationOrigin::Signal })
        )
    ));
    assert_eq!(failure.selected_format().unwrap().as_str(), FORMAT_ID);
    assert_eq!(
        failure.last_completed().phase(),
        ImportPhase::SelectionCompleted
    );
    for (dimension, completed) in [
        (ImportDimension::Faces, CANCEL_AFTER as u64),
        (ImportDimension::TopologyCells, 3 * CANCEL_AFTER as u64),
        (ImportDimension::Vertices, 3),
    ] {
        let row = failure.ledger().dimension(dimension).unwrap();
        assert_eq!(row.consumed(), completed, "{dimension:?}");
        assert_eq!(row.attempted(), completed, "{dimension:?}");
    }
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
    assert_eq!(failure.ledger().current_evidence_bytes(), 0);
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
    let resumed = plan
        .read_bytes(source, &bytes, ImportOperationControl::default())
        .expect("ordinary cancellation must not poison the registration");
    assert_scene(resumed.scene());
    drop((resumed, failure, plan, bytes));
    assert!(evidence.matches_scene(&retained));
    assert_scene(&retained);
}
