use std::alloc::Layout;
use std::sync::atomic::AtomicUsize;
use triomphe::{Arc, UniqueArc};

pub(super) trait FinalOwnerInnerDrop {
    fn release_for_drop(&mut self);

    #[cfg(test)]
    fn record_share(&self);

    #[cfg(test)]
    fn share_call_count(&self) -> usize;
}

pub(super) struct FinalOwnerAllocation<I: FinalOwnerInnerDrop> {
    shared: Option<Arc<I>>,
}

impl<I: FinalOwnerInnerDrop> FinalOwnerAllocation<I> {
    pub(super) fn from_shared(shared: Arc<I>) -> Self {
        Self {
            shared: Some(shared),
        }
    }

    pub(super) fn inner(&self) -> &I {
        match self.shared.as_ref() {
            Some(shared) => shared,
            None => unreachable_final_owner_state(),
        }
    }

    pub(super) fn inner_mut_if_unique(&mut self) -> Option<&mut I> {
        Arc::get_mut(self.shared.as_mut()?)
    }

    pub(super) fn checked_layout_bytes() -> Result<u64, FinalOwnerAllocationError> {
        let (layout, _) = Layout::new::<AtomicUsize>()
            .extend(Layout::new::<I>())
            .map_err(|_| FinalOwnerAllocationError::AccountingOverflow)?;
        u64::try_from(layout.pad_to_align().size())
            .map_err(|_| FinalOwnerAllocationError::AccountingOverflow)
    }

    pub(super) fn share(&self) -> Self {
        let shared = match self.shared.as_ref() {
            Some(shared) => {
                #[cfg(test)]
                shared.record_share();
                Arc::clone(shared)
            }
            None => unreachable_final_owner_state(),
        };
        Self::from_shared(shared)
    }

    pub(super) fn ptr_eq(&self, other: &Self) -> bool {
        match (self.shared.as_ref(), other.shared.as_ref()) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    #[cfg(test)]
    pub(super) fn share_call_count(&self) -> usize {
        self.shared
            .as_ref()
            .map_or(0, |shared| shared.share_call_count())
    }

    #[cfg(test)]
    pub(super) fn allocation_identity(&self) -> *const I {
        self.shared.as_ref().map_or(std::ptr::null(), Arc::as_ptr)
    }

    pub(super) fn into_unique_inner(mut self) -> Option<I> {
        let shared = self.shared.take()?;
        let unique = Arc::into_unique(shared)?;
        Some(UniqueArc::into_inner(unique))
    }

    pub(super) fn try_into_unique_inner(mut self) -> Result<Option<I>, Self> {
        let Some(shared) = self.shared.take() else {
            return Ok(None);
        };
        let unique = match Arc::try_unique(shared) {
            Ok(unique) => unique,
            Err(shared) => {
                self.shared = Some(shared);
                return Err(self);
            }
        };
        Ok(Some(UniqueArc::into_inner(unique)))
    }

    fn release_for_drop(&mut self) {
        let Some(shared) = self.shared.take() else {
            return;
        };
        let Some(unique) = Arc::into_unique(shared) else {
            return;
        };
        let mut inner = UniqueArc::into_inner(unique);
        inner.release_for_drop();
        drop(inner);
    }
}

impl<I: FinalOwnerInnerDrop> Drop for FinalOwnerAllocation<I> {
    fn drop(&mut self) {
        self.release_for_drop();
    }
}

pub(super) fn unreachable_final_owner_state() -> ! {
    panic!("final-owner allocation was observed during destruction")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FinalOwnerAllocationError {
    AccountingOverflow,
    AllocationFailed,
}
