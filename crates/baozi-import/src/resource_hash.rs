use crate::error::ScopeAbortCause;
use crate::ledger::ImportDimension;
use crate::store::ImportStoreHost;
use sha2::{Digest, Sha256};

pub(crate) const HASH_CHUNK_BYTES: usize = 64 * 1024;

/// Hashes immutable bytes while preserving completed work at cancellation boundaries.
pub(crate) fn chunked_sha256(
    host: ImportStoreHost<'_>,
    bytes: &[u8],
    dimension: ImportDimension,
) -> Result<[u8; 32], ScopeAbortCause> {
    let mut hasher = Sha256::new();
    if bytes.is_empty() {
        host.poll()
            .map_err(|cause| cause.into_scope_abort_cause())?;
    }
    for chunk in bytes.chunks(HASH_CHUNK_BYTES) {
        host.poll()
            .map_err(|cause| cause.into_scope_abort_cause())?;
        // Each chunk is at most 64 KiB, so its work amount always fits in u64.
        let work = chunk.len() as u64;
        host.charge_attempted(dimension, work)
            .map_err(|cause| cause.into_scope_abort_cause())?;
        hasher.update(chunk);
        host.charge_consumed(dimension, work)
            .map_err(|cause| cause.into_scope_abort_cause())?;
    }
    Ok(hasher.finalize().into())
}
