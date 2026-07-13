use std::alloc::Layout;
use std::borrow::Borrow;
use std::sync::atomic::AtomicUsize;

use triomphe::Arc;

/// Crate-private fallible shared ownership for immutable canonical storage.
///
/// Keeping the backend private prevents infallible allocation and implicit
/// copy-on-write APIs from leaking into canonical model code.
pub(crate) struct SharedStorage<T> {
    inner: Arc<SharedAllocation<T>>,
}

struct SharedAllocation<T> {
    value: T,
    retained_allocation_bytes: usize,
}

impl<T> SharedStorage<T> {
    pub(crate) fn try_new(value: T) -> Result<Self, SharedStorageError> {
        Self::try_new_prepared(value, 0)
    }

    pub(crate) fn try_new_prepared(
        value: T,
        nested_allocation_bytes: usize,
    ) -> Result<Self, SharedStorageError> {
        let retained_allocation_bytes = Self::checked_layout_footprint()
            .map_err(|SharedStorageFootprintError::LayoutOverflow| {
                SharedStorageError::AccountingOverflow
            })?
            .checked_add(nested_allocation_bytes)
            .ok_or(SharedStorageError::AccountingOverflow)?;

        #[cfg(test)]
        if take_injected_allocation_failure() {
            return Err(SharedStorageError::AllocationFailed);
        }

        let inner = Arc::try_new(SharedAllocation {
            value,
            retained_allocation_bytes,
        })
        .map_err(|_| SharedStorageError::AllocationFailed)?;
        Ok(Self { inner })
    }

    #[cfg(test)]
    pub(crate) fn get_mut(&mut self) -> Option<&mut T> {
        Arc::get_mut(&mut self.inner).map(|allocation| &mut allocation.value)
    }

    pub(crate) fn as_ref(&self) -> &T {
        &self.inner.value
    }

    pub(crate) fn checked_layout_footprint() -> Result<usize, SharedStorageFootprintError> {
        let (layout, _) = Layout::new::<AtomicUsize>()
            .extend(Layout::new::<SharedAllocation<T>>())
            .map_err(|_| SharedStorageFootprintError::LayoutOverflow)?;
        Ok(layout.pad_to_align().size())
    }

    pub(crate) fn checked_allocation_footprint(
        &self,
    ) -> Result<usize, SharedStorageFootprintError> {
        Ok(self.retained_allocation_bytes())
    }

    pub(crate) fn retained_allocation_bytes(&self) -> usize {
        self.inner.retained_allocation_bytes
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
    AccountingOverflow,
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
    fn clone_is_allocation_free_and_preserves_the_cached_footprint() {
        let first = SharedStorage::try_new_prepared(9_u64, 17).unwrap();
        fail_next_allocation_for_test();

        let second = first.clone();

        assert!(first.ptr_eq(&second));
        assert_eq!(
            first.retained_allocation_bytes(),
            second.retained_allocation_bytes()
        );
        assert_eq!(
            SharedStorage::try_new(10_u64).err(),
            Some(SharedStorageError::AllocationFailed),
            "cloning must not consume the next shared-storage allocation attempt"
        );
    }

    #[test]
    fn footprint_includes_reference_count_header_and_aligned_payload() {
        #[repr(align(32))]
        struct Aligned([u8; 3]);

        let storage = SharedStorage::try_new(Aligned([1, 2, 3])).unwrap();
        let footprint = storage.checked_allocation_footprint().unwrap();
        let (expected, _) = Layout::new::<AtomicUsize>()
            .extend(Layout::new::<SharedAllocation<Aligned>>())
            .unwrap();

        assert_eq!(footprint, expected.pad_to_align().size());
        assert!(footprint >= std::mem::size_of::<AtomicUsize>() + 3);
        assert_eq!(storage.as_ref().0, [1, 2, 3]);
    }

    #[test]
    fn prepared_storage_retains_exact_outer_and_nested_allocation_bytes() {
        let nested_bytes = 37;
        let storage = SharedStorage::try_new_prepared(String::from("prepared"), nested_bytes)
            .expect("prepared shared storage should allocate");
        let expected = SharedStorage::<String>::checked_layout_footprint()
            .unwrap()
            .checked_add(nested_bytes)
            .unwrap();

        assert_eq!(storage.retained_allocation_bytes(), expected);
        assert_eq!(storage.clone().retained_allocation_bytes(), expected);
    }

    #[test]
    fn prepared_allocation_failure_drops_the_unpublished_payload() {
        use std::cell::Cell;
        use std::rc::Rc;

        struct DropProbe(Rc<Cell<u32>>);

        impl Drop for DropProbe {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }

        let drops = Rc::new(Cell::new(0));
        fail_next_allocation_for_test();
        let result = SharedStorage::try_new_prepared(DropProbe(Rc::clone(&drops)), 11);

        assert_eq!(result.err(), Some(SharedStorageError::AllocationFailed));
        assert_eq!(drops.get(), 1);
    }

    #[test]
    fn prepared_footprint_overflow_is_typed_and_drops_the_unpublished_payload() {
        use std::cell::Cell;
        use std::rc::Rc;

        struct DropProbe(Rc<Cell<u32>>);

        impl Drop for DropProbe {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }

        let drops = Rc::new(Cell::new(0));
        let result = SharedStorage::try_new_prepared(DropProbe(Rc::clone(&drops)), usize::MAX);

        assert_eq!(result.err(), Some(SharedStorageError::AccountingOverflow));
        assert_eq!(drops.get(), 1);
    }
}
