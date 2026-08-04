//! Process-wide nonces shared by every provenance storage generation.

use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

struct ProcessNonceSource {
    next: AtomicU64,
}

impl ProcessNonceSource {
    const fn new(next: u64) -> Self {
        Self {
            next: AtomicU64::new(next),
        }
    }

    fn issue(&self) -> Result<NonZeroU64, ProcessNonceExhausted> {
        let candidate = self
            .next
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |candidate| {
                NonZeroU64::new(candidate).and_then(|_| candidate.checked_add(1))
            })
            .map_err(|_| ProcessNonceExhausted)?;
        NonZeroU64::new(candidate).ok_or(ProcessNonceExhausted)
    }
}

static PROCESS_NONCES: ProcessNonceSource = ProcessNonceSource::new(1);

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_PROCESS_NONCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProcessNonceExhausted;

pub(crate) fn issue_process_nonce() -> Result<NonZeroU64, ProcessNonceExhausted> {
    #[cfg(test)]
    if FAIL_NEXT_PROCESS_NONCE.with(|flag| flag.replace(false)) {
        return Err(ProcessNonceExhausted);
    }

    PROCESS_NONCES.issue()
}

#[cfg(test)]
pub(crate) fn fail_next_process_nonce_for_test() {
    FAIL_NEXT_PROCESS_NONCE.with(|flag| flag.set(true));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_is_an_exhausted_nonce_source() {
        assert_eq!(
            ProcessNonceSource::new(0).issue(),
            Err(ProcessNonceExhausted),
        );
    }

    #[test]
    fn issuance_stops_before_the_counter_can_wrap() {
        let source = ProcessNonceSource::new(u64::MAX - 1);

        assert_eq!(source.issue().map(NonZeroU64::get), Ok(u64::MAX - 1),);
        assert_eq!(source.issue(), Err(ProcessNonceExhausted));
        assert_eq!(source.issue(), Err(ProcessNonceExhausted));
    }
}
