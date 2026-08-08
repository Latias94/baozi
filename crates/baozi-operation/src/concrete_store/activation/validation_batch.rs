use std::num::NonZeroU64;

const VALIDATION_CHECKPOINT_UNITS: u64 = 256;

/// Local completion prefix for one admitted validation permit.
///
/// Inspectors still run one unit at a time. The batch only delays ledger completion and control
/// polling until a bounded checkpoint or a semantic boundary.
pub(super) struct ValidationWorkBatch {
    unflushed_completed: u64,
    completed_since_poll: u64,
    poll_due: bool,
}

impl ValidationWorkBatch {
    pub(super) const fn new() -> Self {
        Self {
            unflushed_completed: 0,
            completed_since_poll: 0,
            poll_due: true,
        }
    }

    pub(super) const fn poll_due(&self) -> bool {
        self.poll_due
    }

    pub(super) fn note_poll(&mut self) {
        self.completed_since_poll = 0;
        self.poll_due = false;
    }

    pub(super) fn note_completed_unit(&mut self) -> Result<(), ()> {
        self.unflushed_completed = self.unflushed_completed.checked_add(1).ok_or(())?;
        self.completed_since_poll = self.completed_since_poll.checked_add(1).ok_or(())?;
        if self.completed_since_poll >= VALIDATION_CHECKPOINT_UNITS {
            self.poll_due = true;
        }
        Ok(())
    }

    pub(super) const fn effective_remaining(&self, permit_remaining: u64) -> Option<u64> {
        permit_remaining.checked_sub(self.unflushed_completed)
    }

    pub(super) fn take_completed(&mut self) -> Option<NonZeroU64> {
        let completed = NonZeroU64::new(self.unflushed_completed)?;
        self.unflushed_completed = 0;
        Some(completed)
    }
}
