use std::fmt;
use std::mem::size_of;

/// Identifies the allocator-capacity policy qualified by target-specific tests.
pub const CAPACITY_POLICY_REVISION: u32 = 1;

/// A checked, allocation-free plan for one `Vec`-compatible capacity change.
///
/// Stable Rust does not specify an upper bound for allocator-provided spare
/// capacity. Baozi therefore requests a deterministic power-of-two envelope
/// and treats any larger result as an unqualified platform behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapacityPlan {
    original_len: usize,
    original_capacity: usize,
    desired_capacity: usize,
    envelope_capacity: usize,
    element_size: usize,
}

impl CapacityPlan {
    /// Plans capacity for values of `T` without entering the allocator.
    pub fn for_type<T>(
        len: usize,
        capacity: usize,
        desired_capacity: usize,
    ) -> Result<Self, CapacityPlanError> {
        Self::with_element_size(len, capacity, desired_capacity, size_of::<T>())
    }

    /// Plans capacity from an explicit element size.
    pub fn with_element_size(
        len: usize,
        capacity: usize,
        desired_capacity: usize,
        element_size: usize,
    ) -> Result<Self, CapacityPlanError> {
        if len > capacity {
            return Err(CapacityPlanError::InvalidState { len, capacity });
        }

        let desired_capacity = desired_capacity.max(len);
        let envelope_capacity = if element_size == 0 || desired_capacity <= capacity {
            capacity.max(desired_capacity)
        } else {
            desired_capacity
                .checked_next_power_of_two()
                .ok_or(CapacityPlanError::CapacityOverflow)?
        };

        if let Some(maximum_capacity) = (isize::MAX as usize).checked_div(element_size) {
            if envelope_capacity > maximum_capacity {
                return Err(CapacityPlanError::AllocationTooLarge {
                    envelope_capacity,
                    element_size,
                });
            }
            checked_bytes(envelope_capacity, element_size)?;
        }

        Ok(Self {
            original_len: len,
            original_capacity: capacity,
            desired_capacity,
            envelope_capacity,
            element_size,
        })
    }

    pub const fn original_len(self) -> usize {
        self.original_len
    }

    pub const fn original_capacity(self) -> usize {
        self.original_capacity
    }

    pub const fn desired_capacity(self) -> usize {
        self.desired_capacity
    }

    pub const fn envelope_capacity(self) -> usize {
        self.envelope_capacity
    }

    /// Returns the argument for `try_reserve_exact` at the captured length.
    pub const fn allocator_additional(self) -> usize {
        if self.element_size == 0 {
            0
        } else {
            self.envelope_capacity.saturating_sub(self.original_len)
        }
    }

    /// Returns the reservation increase required before allocator entry.
    pub fn envelope_delta_bytes(self) -> Result<u64, CapacityPlanError> {
        if self.element_size == 0 {
            return Ok(0);
        }
        checked_bytes(
            self.envelope_capacity
                .checked_sub(self.original_capacity)
                .ok_or(CapacityPlanError::CapacityOverflow)?,
            self.element_size,
        )
    }

    /// Rejects use after the source container changed.
    pub fn verify_preflight(self, len: usize, capacity: usize) -> Result<(), CapacityPlanError> {
        if len != self.original_len || capacity != self.original_capacity {
            return Err(CapacityPlanError::StalePlan {
                expected_len: self.original_len,
                expected_capacity: self.original_capacity,
                actual_len: len,
                actual_capacity: capacity,
            });
        }
        Ok(())
    }

    /// Verifies qualified allocator behavior and returns retained growth bytes.
    pub fn finish(self, len: usize, capacity: usize) -> Result<u64, CapacityPlanError> {
        if len != self.original_len {
            return Err(CapacityPlanError::StalePlan {
                expected_len: self.original_len,
                expected_capacity: self.original_capacity,
                actual_len: len,
                actual_capacity: capacity,
            });
        }
        if self.element_size == 0 {
            return Ok(0);
        }
        if capacity < self.desired_capacity || capacity > self.envelope_capacity {
            return Err(CapacityPlanError::CapacityPlatformUnqualified {
                policy_revision: CAPACITY_POLICY_REVISION,
                desired_capacity: self.desired_capacity,
                envelope_capacity: self.envelope_capacity,
                actual_capacity: capacity,
            });
        }
        checked_bytes(
            capacity.checked_sub(self.original_capacity).ok_or(
                CapacityPlanError::CapacityPlatformUnqualified {
                    policy_revision: CAPACITY_POLICY_REVISION,
                    desired_capacity: self.desired_capacity,
                    envelope_capacity: self.envelope_capacity,
                    actual_capacity: capacity,
                },
            )?,
            self.element_size,
        )
    }
}

/// A capacity plan could not be created or qualified without allocator drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CapacityPlanError {
    InvalidState {
        len: usize,
        capacity: usize,
    },
    CapacityOverflow,
    AllocationTooLarge {
        envelope_capacity: usize,
        element_size: usize,
    },
    StalePlan {
        expected_len: usize,
        expected_capacity: usize,
        actual_len: usize,
        actual_capacity: usize,
    },
    CapacityPlatformUnqualified {
        policy_revision: u32,
        desired_capacity: usize,
        envelope_capacity: usize,
        actual_capacity: usize,
    },
}

impl fmt::Display for CapacityPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidState { len, capacity } => {
                write!(formatter, "length {len} exceeds capacity {capacity}")
            }
            Self::CapacityOverflow => formatter.write_str("capacity planning overflowed"),
            Self::AllocationTooLarge {
                envelope_capacity,
                element_size,
            } => write!(
                formatter,
                "capacity envelope {envelope_capacity} at {element_size} bytes per element exceeds the allocation contract"
            ),
            Self::StalePlan {
                expected_len,
                expected_capacity,
                actual_len,
                actual_capacity,
            } => write!(
                formatter,
                "capacity plan expected len/capacity {expected_len}/{expected_capacity}, observed {actual_len}/{actual_capacity}"
            ),
            Self::CapacityPlatformUnqualified {
                policy_revision,
                desired_capacity,
                envelope_capacity,
                actual_capacity,
            } => write!(
                formatter,
                "capacity policy revision {policy_revision} required {desired_capacity}..={envelope_capacity}, allocator reported {actual_capacity}"
            ),
        }
    }
}

impl std::error::Error for CapacityPlanError {}

fn checked_bytes(capacity: usize, element_size: usize) -> Result<u64, CapacityPlanError> {
    let bytes = capacity
        .checked_mul(element_size)
        .ok_or(CapacityPlanError::CapacityOverflow)?;
    u64::try_from(bytes).map_err(|_| CapacityPlanError::CapacityOverflow)
}
