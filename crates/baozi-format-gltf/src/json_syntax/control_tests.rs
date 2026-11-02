use super::tests::{admission_plan, glb};
use super::{SCAN_CHUNK_BYTES, Sha256};
use baozi_import::{
    CauseOwner, ImportCause, ImportDimension, ImportOperationControl, ScopeAbortCause,
};
use baozi_io::AssetPath;
use baozi_operation::{CancellationOrigin, CancellationToken, ControlCause};
use sha2::Digest;
use std::cell::RefCell;
use std::error::Error;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

type TestResult = Result<(), Box<dyn Error>>;

struct KeyHashProbe {
    cancel_after: usize,
    token: Option<CancellationToken>,
    cancelled_at: Option<usize>,
    hashed_end: usize,
    digest: [u8; 32],
}

thread_local! {
    static KEY_HASH_PROBE: RefCell<Option<KeyHashProbe>> = const { RefCell::new(None) };
}

struct KeyHashProbeGuard {
    _same_thread: PhantomData<Rc<()>>,
}

impl KeyHashProbeGuard {
    fn new(cancel_after: usize, token: CancellationToken) -> Self {
        KEY_HASH_PROBE.with(|slot| {
            let mut slot = slot.borrow_mut();
            assert!(slot.is_none(), "key hash probes must not overlap");
            *slot = Some(KeyHashProbe {
                cancel_after,
                token: Some(token),
                cancelled_at: None,
                hashed_end: 0,
                digest: [0; 32],
            });
        });
        Self {
            _same_thread: PhantomData,
        }
    }

    fn finish(self) -> KeyHashProbe {
        KEY_HASH_PROBE.with(|slot| slot.borrow_mut().take().expect("active key hash probe"))
    }
}

impl Drop for KeyHashProbeGuard {
    fn drop(&mut self) {
        let _probe = KEY_HASH_PROBE.with(|slot| slot.borrow_mut().take());
    }
}

// Observe the actual digest mutation without polling or completing scanner work.
pub(super) fn key_bytes_hashed(raw_end: usize, hasher: &Sha256) {
    let token = KEY_HASH_PROBE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let probe = slot.as_mut()?;
        if raw_end < probe.cancel_after {
            return None;
        }
        probe.hashed_end = raw_end;
        probe.digest = hasher.clone().finalize().into();
        if raw_end == probe.cancel_after {
            probe.cancelled_at = Some(raw_end);
            probe.token.take()
        } else {
            None
        }
    });
    if let Some(token) = token {
        token.cancel();
    }
}

fn check_key_hash_frontier(scalar_start: usize, cancel_after: usize) -> TestResult {
    let mut json = b"{\"".to_vec();
    json.resize(scalar_start, b'a');
    json.extend_from_slice(b"\xf0\x9f\xa6\x80");
    json.extend_from_slice(b"\":0}");
    let expected_digest: [u8; 32] = Sha256::digest(&json[2..cancel_after]).into();

    for is_glb in [false, true] {
        let document = if is_glb { glb(&json) } else { json.clone() };
        let surface_len = if is_glb {
            document.len() - 20
        } else {
            json.len()
        };
        let source = AssetPath::new(if is_glb {
            "key-frontier.glb"
        } else {
            "key-frontier.gltf"
        })?;
        let inspected = Arc::new(AtomicBool::new(false));
        let plan = admission_plan(|_, _| {}, Arc::clone(&inspected))?;
        let token = CancellationToken::new();
        let probe = KeyHashProbeGuard::new(cancel_after, token.clone());
        let result = plan.read_bytes(
            source.clone(),
            &document,
            ImportOperationControl::default().with_cancellation(token.clone()),
        );
        let physical = probe.finish();
        assert_eq!(physical.cancelled_at, Some(cancel_after));
        assert!(physical.token.is_none());
        assert!(token.is_cancelled());
        assert!(!inspected.load(Ordering::SeqCst));
        let failure = result
            .expect_err("the key hash marker must interrupt JSON admission")
            .into_operation_failure()?;
        assert_eq!(failure.cause().owner(), CauseOwner::Operation);
        assert!(
            matches!(
                failure.cause(),
                ImportCause::Scope(abort)
                    if matches!(abort.cause(), ScopeAbortCause::Control(ControlCause::Cancelled {
                        origin: CancellationOrigin::Signal,
                    }))
            ),
            "expected signal cancellation at the key hash frontier, got {:?}",
            failure.cause()
        );
        let ledger = failure.ledger();
        let decode = ledger.dimension(ImportDimension::DecodeWork).unwrap();
        assert_eq!(decode.attempted(), surface_len as u64);
        assert_eq!(decode.consumed(), cancel_after as u64);
        assert!(decode.consumed() < decode.attempted());
        assert_eq!(ledger.current_live_bytes(), 0);
        assert_eq!(ledger.active_reservations(), 0);
        assert_eq!(ledger.current_evidence_bytes(), 0);
        assert_eq!(ledger.committed_canonical_members(), 0);

        // Reuse the existing admission-only format after removing the physical probe.
        let retry = plan
            .read_bytes(
                source,
                &document,
                ImportOperationControl::default().with_cancellation(CancellationToken::new()),
            )
            .expect_err("the inspection format stops after successful admission")
            .into_operation_failure()?;
        assert!(inspected.load(Ordering::SeqCst));
        let ledger = retry.ledger();
        assert_eq!(
            ledger
                .dimension(ImportDimension::DecodeWork)
                .unwrap()
                .consumed(),
            surface_len as u64
        );
        assert_eq!(ledger.current_live_bytes(), 0);
        assert_eq!(ledger.active_reservations(), 0);
        assert_eq!(ledger.current_evidence_bytes(), 0);
        KEY_HASH_PROBE.with(|slot| assert!(slot.borrow().is_none()));

        assert_eq!(
            physical.hashed_end as u64,
            decode.consumed(),
            "the digest mutated beyond the completed raw frontier (GLB: {is_glb})"
        );
        assert_eq!(physical.digest, expected_digest);
    }
    Ok(())
}

#[test]
fn non_ascii_key_hash_checks_before_crossing_the_raw_frontier() -> TestResult {
    // The preceding ASCII byte has already been consumed and hashed when the token is set.
    check_key_hash_frontier(SCAN_CHUNK_BYTES - 1, SCAN_CHUNK_BYTES - 1)
}

#[test]
fn non_ascii_key_hash_completes_before_the_post_frontier_poll() -> TestResult {
    check_key_hash_frontier(SCAN_CHUNK_BYTES - 4, SCAN_CHUNK_BYTES)
}

#[test]
fn key_hash_probe_is_reset_on_unwind() {
    let unwind = std::panic::catch_unwind(|| {
        let _probe = KeyHashProbeGuard::new(SCAN_CHUNK_BYTES - 1, CancellationToken::new());
        panic!("exercise scoped key hash probe cleanup");
    });
    assert!(unwind.is_err());
    KEY_HASH_PROBE.with(|slot| assert!(slot.borrow().is_none()));
    let probe = KeyHashProbeGuard::new(SCAN_CHUNK_BYTES - 1, CancellationToken::new());
    assert_eq!(probe.finish().hashed_end, 0);
}
