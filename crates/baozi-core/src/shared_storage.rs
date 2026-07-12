use std::alloc::Layout;
use std::borrow::Borrow;
use std::sync::atomic::AtomicUsize;

use triomphe::Arc;

/// Crate-private fallible shared ownership for immutable canonical storage.
///
/// Keeping the backend private prevents infallible allocation and implicit
/// copy-on-write APIs from leaking into canonical model code.
pub(crate) struct SharedStorage<T> {
    inner: Arc<T>,
}

impl<T> SharedStorage<T> {
    pub(crate) fn try_new(value: T) -> Result<Self, SharedStorageError> {
        #[cfg(test)]
        if take_injected_allocation_failure() {
            return Err(SharedStorageError::AllocationFailed);
        }

        let inner = Arc::try_new(value).map_err(|_| SharedStorageError::AllocationFailed)?;
        Ok(Self { inner })
    }

    pub(crate) fn get_mut(&mut self) -> Option<&mut T> {
        Arc::get_mut(&mut self.inner)
    }

    pub(crate) fn as_ref(&self) -> &T {
        self.inner.borrow()
    }

    pub(crate) fn checked_layout_footprint() -> Result<usize, SharedStorageFootprintError> {
        let (layout, _) = Layout::new::<AtomicUsize>()
            .extend(Layout::new::<T>())
            .map_err(|_| SharedStorageFootprintError::LayoutOverflow)?;
        Ok(layout.pad_to_align().size())
    }

    pub(crate) fn checked_allocation_footprint(
        &self,
    ) -> Result<usize, SharedStorageFootprintError> {
        Self::checked_layout_footprint()
    }

    #[cfg(test)]
    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.as_ref(), other.as_ref())
    }
}

impl<T> Clone for SharedStorage<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Borrow<T> for SharedStorage<T> {
    fn borrow(&self) -> &T {
        self.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SharedStorageError {
    AllocationFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SharedStorageFootprintError {
    LayoutOverflow,
}

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_ALLOCATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn fail_next_allocation_for_test() {
    FAIL_NEXT_ALLOCATION.with(|flag| flag.set(true));
}

#[cfg(test)]
fn take_injected_allocation_failure() -> bool {
    FAIL_NEXT_ALLOCATION.with(|flag| flag.replace(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocation_failure_is_typed_and_does_not_publish_storage() {
        fail_next_allocation_for_test();
        assert_eq!(
            SharedStorage::try_new(7_u64).err(),
            Some(SharedStorageError::AllocationFailed)
        );
    }

    #[test]
    fn clone_shares_storage_and_unique_access_requires_one_owner() {
        let mut first = SharedStorage::try_new(String::from("canonical")).unwrap();
        assert_eq!(
            first.get_mut().map(|value| value.as_str()),
            Some("canonical")
        );

        let second = first.clone();
        assert!(first.ptr_eq(&second));
        assert!(first.get_mut().is_none());
        drop(second);
        assert_eq!(
            first.get_mut().map(|value| value.as_str()),
            Some("canonical")
        );
    }

    #[test]
    fn footprint_includes_reference_count_header_and_aligned_payload() {
        #[repr(align(32))]
        struct Aligned([u8; 3]);

        let storage = SharedStorage::try_new(Aligned([1, 2, 3])).unwrap();
        let footprint = storage.checked_allocation_footprint().unwrap();
        let (expected, _) = Layout::new::<AtomicUsize>()
            .extend(Layout::new::<Aligned>())
            .unwrap();

        assert_eq!(footprint, expected.pad_to_align().size());
        assert!(footprint >= std::mem::size_of::<AtomicUsize>() + 3);
        assert_eq!(storage.as_ref().0, [1, 2, 3]);
    }
}
