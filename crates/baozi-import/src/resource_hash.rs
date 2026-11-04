use crate::error::ScopeAbortCause;
use crate::ledger::ImportDimension;
use crate::store::ImportRootExecution;
use sha2::{Digest, Sha256};

pub(crate) const HASH_CHUNK_BYTES: usize = 64 * 1024;

/// Hashes immutable bytes while preserving completed work at cancellation boundaries.
pub(crate) fn chunked_sha256(
    exec: &mut ImportRootExecution<'_, '_>,
    bytes: &[u8],
    dimension: ImportDimension,
) -> Result<[u8; 32], ScopeAbortCause> {
    let mut hasher = Sha256::new();
    exec.poll()
        .map_err(|cause| cause.into_scope_abort_cause())?;
    if bytes.is_empty() {
        return Ok(hasher.finalize().into());
    }
    let group = exec
        .bind_group([dimension])
        .map_err(|cause| cause.into_scope_abort_cause())?;
    for (index, chunk) in bytes.chunks(HASH_CHUNK_BYTES).enumerate() {
        if index != 0 {
            exec.poll()
                .map_err(|cause| cause.into_scope_abort_cause())?;
        }
        // Each chunk is at most 64 KiB, so its work amount always fits in u64.
        let work = chunk.len() as u64;
        exec.charge_attempted_group(&group, [work])
            .map_err(|cause| cause.into_scope_abort_cause())?;
        hasher.update(chunk);
        exec.complete_observed_group(&group, [work])
            .map_err(|cause| cause.into_scope_abort_cause())?;
    }
    Ok(hasher.finalize().into())
}
