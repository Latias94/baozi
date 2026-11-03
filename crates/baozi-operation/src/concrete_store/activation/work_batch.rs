use std::num::NonZeroU64;

const WORK_CHECKPOINT_UNITS: u64 = 256;

/// Local completion prefix for one admitted work permit.
///
/// Callers still perform each semantic action at its natural granularity. The batch only delays
/// ledger completion and control polling until a bounded checkpoint or a semantic boundary.
pub(super) struct CompletionPrefixBatch {
    unflushed_completed: u64,
    completed_since_poll: u64,
    poll_due: bool,
}

impl CompletionPrefixBatch {
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

    pub(super) const fn units_until_checkpoint(&self) -> u64 {
        if self.poll_due {
            0
        } else {
            WORK_CHECKPOINT_UNITS - self.completed_since_poll
        }
    }

    pub(super) const fn unflushed_completed(&self) -> u64 {
        self.unflushed_completed
    }

    pub(super) fn note_completed(&mut self, completed: NonZeroU64) -> Result<(), ()> {
        if self.poll_due {
            return Err(());
        }
        let completed = completed.get();
        let unflushed_completed = self.unflushed_completed.checked_add(completed).ok_or(())?;
        let completed_since_poll = self
            .completed_since_poll
            .checked_add(completed)
            .filter(|completed| *completed <= WORK_CHECKPOINT_UNITS)
            .ok_or(())?;
        self.unflushed_completed = unflushed_completed;
        self.completed_since_poll = completed_since_poll;
        if self.completed_since_poll == WORK_CHECKPOINT_UNITS {
            self.poll_due = true;
        }
        Ok(())
    }

    pub(super) fn note_completed_unit(&mut self) -> Result<(), ()> {
        self.note_completed(NonZeroU64::MIN)
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
