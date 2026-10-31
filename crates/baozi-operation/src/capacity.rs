use std::fmt;
use std::mem::size_of;

/// Identifies the deterministic capacity-request policy used by this crate.
///
/// This revision describes Baozi's request and post-allocation checks only. It is not a claim
/// about the allocator selected by the final binary, its toolchain, or its target. Those facts are
/// collected by the repository qualification probe and never enter an operation or semantic
/// digest.
pub const CAPACITY_POLICY_REVISION: u32 = 2;

/// Identifies the concrete-store capacity/layout contract covered by the repository qualification
/// probe.
///
/// The value is deliberately separate from semantic catalog and terminal-evidence revisions. It
/// is a crate-internal qualification subject, not a stable wire or Scene identity.
#[allow(
    dead_code,
    reason = "reserved as a non-runtime evidence anchor for the repository qualification module"
)]
pub(crate) const CONCRETE_STORE_CAPACITY_REVISION: u32 = 1;

/// Non-semantic subject used to match repository qualification evidence to the implementation.
///
/// The fields and constructor remain private so callers cannot mint a different subject and turn
/// it into a runtime qualification claim. A matching subject still carries no allocator,
/// toolchain, target, or RSS authority; those are external evidence dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "reserved as a non-runtime evidence anchor for the repository qualification module"
)]
pub(crate) struct CapacityQualificationSubject {
    concrete_store_revision: u32,
    capacity_policy_revision: u32,
}

#[allow(
    dead_code,
    reason = "reserved as a non-runtime evidence anchor for the repository qualification module"
)]
impl CapacityQualificationSubject {
    pub(crate) const CURRENT: Self = Self {
        concrete_store_revision: CONCRETE_STORE_CAPACITY_REVISION,
        capacity_policy_revision: CAPACITY_POLICY_REVISION,
    };

    pub(crate) const fn concrete_store_revision(self) -> u32 {
        self.concrete_store_revision
    }

    pub(crate) const fn capacity_policy_revision(self) -> u32 {
        self.capacity_policy_revision
    }

    /// Checks an evidence tuple without accepting it as a runtime capability.
    pub(crate) const fn matches(self, concrete_store_revision: u32, policy_revision: u32) -> bool {
        self.concrete_store_revision == concrete_store_revision
            && self.capacity_policy_revision == policy_revision
    }
}

/// A checked, allocation-free plan for one `Vec`-compatible capacity change.
///
/// Stable Rust does not specify an upper bound for allocator-provided spare capacity. Baozi
/// therefore admits a deterministic power-of-two envelope, requests only the desired capacity,
/// and treats any larger observed result as a capacity-envelope violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapacityPlan {
    original_len: usize,
    original_capacity: usize,
    desired_capacity: usize,
    envelope_capacity: usize,
    element_size: usize,
}

/// A source-container capacity plan paired with its complete replacement allocation.
///
/// The source plan preserves the caller's minimum required capacity. The candidate plan promotes
/// that requirement to the source envelope, so an empty replacement requests geometric spare
/// capacity while retaining the single meaning of [`CapacityPlan::allocator_additional`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplacementCapacityPlan {
    source: CapacityPlan,
    candidate: CapacityPlan,
}

/// A logical capacity requirement paired with an amortized in-place allocation target.
///
/// The requirement preserves the caller's minimum desired capacity. The allocation plan promotes
/// that minimum to the deterministic envelope, preventing repeated `len + 1` growth from becoming
/// quadratic while keeping [`CapacityPlan::allocator_additional`] single-purpose.
///
/// This plan does not make allocator growth transactional. Once an in-place allocation succeeds,
/// a later envelope check cannot restore the source container's previous capacity. Storage that
/// must preserve its original allocation on any qualification failure must build and validate a
/// separate candidate with [`ReplacementCapacityPlan`] before replacing the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrowableCapacityPlan {
    requirement: CapacityPlan,
    allocation: CapacityPlan,
}

impl GrowableCapacityPlan {
    /// Plans amortized in-place growth for values of `T`.
    pub fn for_type<T>(
        len: usize,
        capacity: usize,
        desired_capacity: usize,
    ) -> Result<Self, CapacityPlanError> {
        Self::with_element_size(len, capacity, desired_capacity, size_of::<T>())
    }

    /// Plans amortized in-place growth using an explicit element size.
    pub fn with_element_size(
        len: usize,
        capacity: usize,
        desired_capacity: usize,
        element_size: usize,
    ) -> Result<Self, CapacityPlanError> {
        let requirement =
            CapacityPlan::with_element_size(len, capacity, desired_capacity, element_size)?;
        let allocation = CapacityPlan::with_element_size(
            len,
            capacity,
            requirement.envelope_capacity(),
            element_size,
        )?;
        Ok(Self {
            requirement,
            allocation,
        })
    }

    /// Returns the caller's minimum logical capacity requirement.
    pub const fn desired_capacity(self) -> usize {
        self.requirement.desired_capacity()
    }

    /// Returns the complete geometric capacity envelope admitted before allocator entry.
    pub const fn envelope_capacity(self) -> usize {
        self.allocation.envelope_capacity()
    }

    /// Returns the request passed to `try_reserve_exact` for the admitted envelope.
    pub const fn allocator_additional(self) -> usize {
        self.allocation.allocator_additional()
    }

    /// Verifies that the plan still describes the same live source.
    pub fn verify_source(self, len: usize, capacity: usize) -> Result<(), CapacityPlanError> {
        self.requirement.verify_preflight(len, capacity)?;
        self.allocation.verify_preflight(len, capacity)
    }

    /// Verifies allocator output against the admitted envelope and returns growth bytes.
    pub fn finish(self, len: usize, capacity: usize) -> Result<u64, CapacityPlanError> {
        self.allocation.finish(len, capacity)
    }
}

impl ReplacementCapacityPlan {
    /// Plans a complete growable replacement for values of `T`.
    pub fn for_type<T>(
        source_len: usize,
        source_capacity: usize,
        desired_capacity: usize,
    ) -> Result<Self, CapacityPlanError> {
        Self::with_element_size(
            source_len,
            source_capacity,
            desired_capacity,
            size_of::<T>(),
        )
    }

    /// Plans a complete growable replacement using an explicit element size.
    pub fn with_element_size(
        source_len: usize,
        source_capacity: usize,
        desired_capacity: usize,
        element_size: usize,
    ) -> Result<Self, CapacityPlanError> {
        let source = CapacityPlan::with_element_size(
            source_len,
            source_capacity,
            desired_capacity,
            element_size,
        )?;
        if source.desired_capacity() <= source.original_capacity() {
            return Err(CapacityPlanError::ReplacementNotRequired {
                desired_capacity: source.desired_capacity(),
                current_capacity: source.original_capacity(),
            });
        }
        let candidate =
            CapacityPlan::with_element_size(0, 0, source.envelope_capacity(), element_size)?;
        Ok(Self { source, candidate })
    }

    /// Returns the plan that qualifies the current live container.
    pub const fn source(self) -> CapacityPlan {
        self.source
    }

    /// Returns the plan for the empty replacement candidate.
    pub const fn candidate(self) -> CapacityPlan {
        self.candidate
    }

    /// Verifies that the live source still matches the captured state.
    pub fn verify_source(self, len: usize, capacity: usize) -> Result<(), CapacityPlanError> {
        self.source.verify_preflight(len, capacity)
    }

    /// Returns the complete replacement envelope that must be admitted alongside the source.
    pub fn candidate_envelope_bytes(self) -> Result<u64, CapacityPlanError> {
        self.candidate.envelope_delta_bytes()
    }
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

    /// Returns the requested growth argument for `try_reserve_exact` at the captured length.
    ///
    /// The request is intentionally based on `desired_capacity`, not the conservative envelope.
    /// The full envelope is admitted before allocator entry; `finish` accepts an allocator result
    /// anywhere in `desired_capacity..=envelope_capacity` and rejects over-envelope growth.
    pub const fn allocator_additional(self) -> usize {
        if self.element_size == 0 {
            0
        } else {
            self.desired_capacity.saturating_sub(self.original_len)
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

    /// Verifies that observed capacity stays inside the admitted envelope and returns growth bytes.
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
            return Err(CapacityPlanError::CapacityEnvelopeViolated {
                policy_revision: CAPACITY_POLICY_REVISION,
                desired_capacity: self.desired_capacity,
                envelope_capacity: self.envelope_capacity,
                actual_capacity: capacity,
            });
        }
        checked_bytes(
            capacity.checked_sub(self.original_capacity).ok_or(
                CapacityPlanError::CapacityEnvelopeViolated {
                    policy_revision: CAPACITY_POLICY_REVISION,
                    desired_capacity: self.desired_capacity,
                    envelope_capacity: self.envelope_capacity,
                    actual_capacity: capacity,
                },
            )?,
            self.element_size,
        )
    }

    /// Finishes a plan while retaining storage-before-owner cleanup on failure.
    ///
    /// The returned storage is still owned by the caller on success. On any qualification error,
    /// this function drops `storage` before returning, so a surrounding operation can release its
    /// accounting owner after the backing storage is gone. It does not perform accounting itself.
    pub(crate) fn finish_owned<T>(
        self,
        storage: T,
        len: usize,
        capacity: usize,
    ) -> Result<(T, u64), CapacityPlanError> {
        match self.finish(len, capacity) {
            Ok(bytes) => Ok((storage, bytes)),
            Err(error) => {
                drop(storage);
                Err(error)
            }
        }
    }
}

/// A capacity plan could not be created or completed inside its admitted envelope.
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
    ReplacementNotRequired {
        desired_capacity: usize,
        current_capacity: usize,
    },
    CapacityEnvelopeViolated {
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
            Self::ReplacementNotRequired {
                desired_capacity,
                current_capacity,
            } => write!(
                formatter,
                "complete replacement requires growth, but desired capacity {desired_capacity} fits current capacity {current_capacity}"
            ),
            Self::CapacityEnvelopeViolated {
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::{CapacityPlan, CapacityQualificationSubject};

    #[test]
    fn qualification_subject_rejects_mismatch_and_unknown_revisions() {
        // The subject is intentionally not a runtime capability. It only matches exact repository
        // evidence and rejects both stale and unknown tuples.
        let current = CapacityQualificationSubject::CURRENT;
        assert!(current.matches(
            current.concrete_store_revision(),
            current.capacity_policy_revision()
        ));
        assert!(!current.matches(
            current.concrete_store_revision() + 1,
            current.capacity_policy_revision()
        ));
        assert!(!current.matches(
            current.concrete_store_revision(),
            current.capacity_policy_revision() + 1
        ));
    }

    #[derive(Debug)]
    struct DropProbe {
        label: &'static str,
        events: Rc<RefCell<Vec<&'static str>>>,
    }

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.events.borrow_mut().push(self.label);
        }
    }

    #[test]
    fn over_envelope_failure_drops_storage_before_surrounding_owner() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let storage = DropProbe {
            label: "storage",
            events: Rc::clone(&events),
        };
        let owner = DropProbe {
            label: "owner",
            events: Rc::clone(&events),
        };
        let plan = CapacityPlan::for_type::<u8>(0, 0, 3).expect("capacity plan");

        let error = plan
            .finish_owned(storage, 0, 5)
            .expect_err("over-envelope capacity must be rejected");
        assert!(matches!(
            error,
            super::CapacityPlanError::CapacityEnvelopeViolated { .. }
        ));
        assert_eq!(&*events.borrow(), &["storage"]);

        drop(owner);
        assert_eq!(&*events.borrow(), &["storage", "owner"]);
    }
}
