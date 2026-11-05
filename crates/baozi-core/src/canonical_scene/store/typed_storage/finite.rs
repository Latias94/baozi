//! Contiguous geometry floats qualified at their only mutation boundary.

use std::ops::Deref;

use baozi_operation::CapacityPlan;

use super::build::{BuildResult, allocate, finish};

mod sealed {
    pub trait Sealed {}
    impl Sealed for f32 {}
    impl Sealed for f64 {}
}

pub(in crate::canonical_scene::store) trait FiniteScalar:
    sealed::Sealed + Copy
{
    fn is_finite(self) -> bool;
}

impl FiniteScalar for f32 {
    fn is_finite(self) -> bool {
        self.is_finite()
    }
}

impl FiniteScalar for f64 {
    fn is_finite(self) -> bool {
        self.is_finite()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum FiniteWriteError {
    Capacity,
    NonFinite { scalar: usize },
}

/// No mutable slice or raw-vector conversion escapes this module.
#[derive(Debug)]
pub(in crate::canonical_scene::store) struct FiniteScalarPool<T> {
    values: Vec<T>,
}

impl<T: FiniteScalar> FiniteScalarPool<T> {
    pub(in crate::canonical_scene::store) fn allocate<E>(
        plan: CapacityPlan,
    ) -> BuildResult<Self, E> {
        Ok(Self {
            values: allocate(plan)?,
        })
    }

    pub(in crate::canonical_scene::store) fn capacity(&self) -> usize {
        self.values.capacity()
    }

    /// The caller invokes this inside its existing admitted scalar action.
    pub(in crate::canonical_scene::store) fn try_push(
        &mut self,
        value: T,
        plan: CapacityPlan,
    ) -> Result<(), FiniteWriteError> {
        self.check_room(1, plan)?;
        if !value.is_finite() {
            return Err(FiniteWriteError::NonFinite { scalar: 0 });
        }
        self.values.push(value);
        Ok(())
    }

    /// One existing tuple action, bounded by the attribute's u8 lane width.
    pub(in crate::canonical_scene::store) fn try_extend_tuple(
        &mut self,
        values: &[T],
        plan: CapacityPlan,
    ) -> Result<(), FiniteWriteError> {
        if values.is_empty() || values.len() > usize::from(u8::MAX) {
            return Err(FiniteWriteError::Capacity);
        }
        self.check_room(values.len(), plan)?;
        if let Some(scalar) = values.iter().position(|value| !value.is_finite()) {
            return Err(FiniteWriteError::NonFinite { scalar });
        }
        self.values.extend_from_slice(values);
        Ok(())
    }

    fn check_room(&self, count: usize, plan: CapacityPlan) -> Result<(), FiniteWriteError> {
        match self.values.len().checked_add(count) {
            Some(end) if end <= plan.desired_capacity() && end <= self.capacity() => Ok(()),
            _ => Err(FiniteWriteError::Capacity),
        }
    }

    pub(in crate::canonical_scene::store) fn finish<E>(
        &self,
        plan: CapacityPlan,
    ) -> BuildResult<(), E> {
        finish(&self.values, plan)
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn fixture(values: &[T]) -> Self {
        use super::build::capacity;
        let plan = capacity::<T>(values.len() as u64).unwrap();
        let mut pool = Self::allocate::<std::convert::Infallible>(plan).unwrap();
        for &value in values {
            pool.try_push(value, plan).unwrap();
        }
        pool
    }
}

impl<T> Deref for FiniteScalarPool<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.values
    }
}

#[cfg(test)]
mod tests {
    use super::super::build::capacity;
    use super::*;

    fn rejects_scalar<T: FiniteScalar + std::fmt::Debug + PartialEq>(finite: T, invalid: [T; 3]) {
        let plan = capacity::<T>(2).unwrap();
        let mut pool = FiniteScalarPool::allocate::<std::convert::Infallible>(plan).unwrap();
        pool.try_push(finite, plan).unwrap();
        for value in invalid {
            assert_eq!(
                pool.try_push(value, plan),
                Err(FiniteWriteError::NonFinite { scalar: 0 }),
            );
            assert_eq!(&*pool, &[finite]);
        }
        pool.try_push(finite, plan).unwrap();
        pool.finish::<std::convert::Infallible>(plan).unwrap();
        assert_eq!(pool.try_push(finite, plan), Err(FiniteWriteError::Capacity));
    }

    #[test]
    fn scalar_rejection_keeps_the_initialized_prefix_for_both_widths() {
        rejects_scalar(-0.0_f32, [f32::NAN, f32::INFINITY, f32::NEG_INFINITY]);
        rejects_scalar(-0.0_f64, [f64::NAN, f64::INFINITY, f64::NEG_INFINITY]);
    }

    #[test]
    fn tuple_rejection_is_atomic_and_respects_the_planned_end() {
        let plan = capacity::<f64>(4).unwrap();
        let mut pool = FiniteScalarPool::allocate::<std::convert::Infallible>(plan).unwrap();
        pool.try_push(-0.0, plan).unwrap();
        assert_eq!(
            pool.try_extend_tuple(&[1.0, 2.0, f64::NAN], plan),
            Err(FiniteWriteError::NonFinite { scalar: 2 }),
        );
        assert_eq!(pool.len(), 1);
        assert_eq!(pool[0].to_bits(), (-0.0_f64).to_bits());
        assert_eq!(
            pool.try_extend_tuple(&[1.0; 4], plan),
            Err(FiniteWriteError::Capacity),
        );
        pool.try_extend_tuple(&[f64::from_bits(1), 1.0e200, -1.0], plan)
            .unwrap();
        pool.finish::<std::convert::Infallible>(plan).unwrap();
        assert_eq!(pool[1].to_bits(), 1);
        assert_eq!(pool[2].to_bits(), 1.0e200_f64.to_bits());
    }

    #[test]
    fn writes_obey_the_plan_even_with_spare_allocation_and_bound_tuple_width() {
        let allocation = capacity::<f32>(256).unwrap();
        let one = capacity::<f32>(1).unwrap();
        let mut pool = FiniteScalarPool::allocate::<std::convert::Infallible>(allocation).unwrap();
        let address = pool.as_ptr();
        pool.try_extend_tuple(&[1.0], one).unwrap();
        assert_eq!(pool.try_push(2.0, one), Err(FiniteWriteError::Capacity));
        assert_eq!(pool.len(), 1);
        assert_eq!(
            pool.try_extend_tuple(&[], allocation),
            Err(FiniteWriteError::Capacity)
        );
        assert_eq!(
            pool.try_extend_tuple(&[1.0; 256], allocation),
            Err(FiniteWriteError::Capacity)
        );
        pool.try_extend_tuple(&[1.0; 255], allocation).unwrap();
        pool.finish::<std::convert::Infallible>(allocation).unwrap();
        assert_eq!(pool.as_ptr(), address);
    }
}
