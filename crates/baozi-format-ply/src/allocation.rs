use baozi_core::{OrderedMap, OrderedMapError};
use baozi_import::{
    CapacityPlan, CapacityPlanError, ImportLiveReservation, ImportStorageClass, ParseResult,
    ParseScope, ScopedSceneBuilder,
};
use std::fmt::Write as _;

const STRING_COPY_CHUNK_BYTES: usize = 4 * 1024;

fn append_string_chunked(
    scope: &ParseScope<'_>,
    output: &mut String,
    value: &str,
) -> ParseResult<()> {
    let mut start = 0usize;
    while start < value.len() {
        scope.poll()?;
        let mut end = start
            .checked_add(STRING_COPY_CHUNK_BYTES)
            .map_or(value.len(), |end| end.min(value.len()));
        while end > start && !value.is_char_boundary(end) {
            end -= 1;
        }
        output.push_str(&value[start..end]);
        start = end;
    }
    Ok(())
}

struct HeldReservation<'scope> {
    reservation: ImportLiveReservation<'scope>,
    payload: bool,
}

pub(crate) struct LiveStorage<'scope> {
    class: ImportStorageClass,
    reservations: Vec<HeldReservation<'scope>>,
}

impl<'scope> LiveStorage<'scope> {
    pub(crate) const fn new(class: ImportStorageClass) -> Self {
        Self {
            class,
            reservations: Vec::new(),
        }
    }

    pub(crate) fn reserve_vec<T>(
        &mut self,
        scope: &'scope ParseScope<'_>,
        values: &mut Vec<T>,
        additional: usize,
        resource: &'static str,
    ) -> ParseResult<()> {
        let required = values
            .len()
            .checked_add(additional)
            .ok_or_else(|| scope.allocation_failure(resource))?;
        if required <= values.capacity() {
            return Ok(());
        }

        let old_capacity = values.capacity();
        let target_capacity = old_capacity.saturating_mul(2).max(required).max(4);
        let plan =
            capacity_plan::<T>(scope, values.len(), old_capacity, target_capacity, resource)?;
        self.ensure_slots(scope, 1)?;
        let reserved_bytes = plan
            .envelope_delta_bytes()
            .map_err(|error| capacity_plan_failure(scope, error, resource))?;
        let mut reservation = scope.reserve_live(self.class, reserved_bytes)?;
        plan.verify_preflight(values.len(), values.capacity())
            .map_err(|error| capacity_plan_failure(scope, error, resource))?;
        values
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| scope.allocation_failure(resource))?;
        let actual_bytes = plan
            .finish(values.len(), values.capacity())
            .map_err(|error| capacity_plan_failure(scope, error, resource))?;
        debug_assert!(actual_bytes <= reserved_bytes);
        scope.resize_live(&mut reservation, actual_bytes)?;
        self.reservations.push(HeldReservation {
            reservation,
            payload: true,
        });
        Ok(())
    }

    pub(crate) fn copy_string(
        &mut self,
        scope: &'scope ParseScope<'_>,
        value: &str,
        resource: &'static str,
    ) -> ParseResult<String> {
        self.allocate_string(scope, value.len(), resource, |output| {
            append_string_chunked(scope, output, value)
        })
    }

    pub(crate) fn empty_string_with_capacity(
        &mut self,
        scope: &'scope ParseScope<'_>,
        length: usize,
        resource: &'static str,
    ) -> ParseResult<String> {
        if length == 0 {
            return Ok(String::new());
        }
        self.allocate_string(scope, length, resource, |_| Ok(()))
    }

    pub(crate) fn copy_prefixed_string(
        &mut self,
        scope: &'scope ParseScope<'_>,
        prefix: &str,
        value: &str,
        resource: &'static str,
    ) -> ParseResult<String> {
        let length = prefix
            .len()
            .checked_add(value.len())
            .ok_or_else(|| scope.allocation_failure(resource))?;
        self.allocate_string(scope, length, resource, |output| {
            append_string_chunked(scope, output, prefix)?;
            append_string_chunked(scope, output, value)
        })
    }

    pub(crate) fn copy_indexed_string(
        &mut self,
        scope: &'scope ParseScope<'_>,
        prefix: &str,
        index: usize,
        resource: &'static str,
    ) -> ParseResult<String> {
        let length = prefix
            .len()
            .checked_add(decimal_digits(index))
            .ok_or_else(|| scope.allocation_failure(resource))?;
        self.allocate_string(scope, length, resource, |output| {
            append_string_chunked(scope, output, prefix)?;
            write!(output, "{index}").map_err(|_| scope.allocation_failure(resource))
        })
    }

    pub(crate) fn reserve_ordered_map<K, V>(
        &mut self,
        scope: &'scope ParseScope<'_>,
        map: &mut OrderedMap<K, V>,
        additional: usize,
        resource: &'static str,
    ) -> ParseResult<()> {
        let required = map
            .len()
            .checked_add(additional)
            .ok_or_else(|| scope.allocation_failure(resource))?;
        if required <= map.capacity() {
            return Ok(());
        }

        let plan = map
            .capacity_plan(additional)
            .map_err(|error| ordered_map_failure(scope, error, resource))?;
        self.ensure_slots(scope, 1)?;
        let admitted = plan
            .envelope_delta_bytes()
            .map_err(|error| ordered_map_failure(scope, error, resource))?;
        let mut reservation = scope.reserve_live(self.class, admitted)?;
        let actual = match map.apply_capacity_plan(plan) {
            Ok(actual) => actual,
            Err(error) => {
                self.reservations.push(HeldReservation {
                    reservation,
                    payload: true,
                });
                return Err(ordered_map_failure(scope, error, resource));
            }
        };
        debug_assert!(actual <= admitted);
        if let Err(failure) = scope.resize_live(&mut reservation, actual) {
            self.reservations.push(HeldReservation {
                reservation,
                payload: true,
            });
            return Err(failure);
        }
        self.reservations.push(HeldReservation {
            reservation,
            payload: true,
        });
        Ok(())
    }

    pub(crate) fn adopt(self, builder: &mut ScopedSceneBuilder<'_, '_>) -> ParseResult<()> {
        let mut reservations = self.reservations;
        let mut index = 0;
        while index < reservations.len() {
            if !reservations[index].payload {
                index += 1;
                continue;
            }
            let held = reservations.swap_remove(index);
            builder.adopt_reservation(held.reservation)?;
        }
        Ok(())
    }

    fn allocate_string(
        &mut self,
        scope: &'scope ParseScope<'_>,
        length: usize,
        resource: &'static str,
        write_value: impl FnOnce(&mut String) -> ParseResult<()>,
    ) -> ParseResult<String> {
        let mut output = String::new();
        let plan = capacity_plan::<u8>(scope, output.len(), output.capacity(), length, resource)?;
        self.ensure_slots(scope, 1)?;
        let admitted = plan
            .envelope_delta_bytes()
            .map_err(|error| capacity_plan_failure(scope, error, resource))?;
        let mut reservation = scope.reserve_live(self.class, admitted)?;
        plan.verify_preflight(output.len(), output.capacity())
            .map_err(|error| capacity_plan_failure(scope, error, resource))?;
        output
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| scope.allocation_failure(resource))?;
        let actual = plan
            .finish(output.len(), output.capacity())
            .map_err(|error| capacity_plan_failure(scope, error, resource))?;
        debug_assert!(actual <= admitted);
        scope.resize_live(&mut reservation, actual)?;
        self.reservations.push(HeldReservation {
            reservation,
            payload: true,
        });
        write_value(&mut output)?;
        Ok(output)
    }

    fn ensure_slots(
        &mut self,
        scope: &'scope ParseScope<'_>,
        required_free_slots: usize,
    ) -> ParseResult<()> {
        if self.reservations.capacity() - self.reservations.len() >= required_free_slots {
            return Ok(());
        }

        let old_capacity = self.reservations.capacity();
        let minimum_capacity = self
            .reservations
            .len()
            .checked_add(required_free_slots)
            .and_then(|value| value.checked_add(2))
            .ok_or_else(|| scope.allocation_failure("ply-reservation-list"))?;
        let target_capacity = old_capacity.saturating_mul(2).max(minimum_capacity).max(8);
        let plan = capacity_plan::<HeldReservation<'_>>(
            scope,
            self.reservations.len(),
            old_capacity,
            target_capacity,
            "ply-reservation-list",
        )?;
        let reserved_bytes = plan
            .envelope_delta_bytes()
            .map_err(|error| capacity_plan_failure(scope, error, "ply-reservation-list"))?;
        let mut reservation = scope.reserve_live(self.class, reserved_bytes)?;
        plan.verify_preflight(self.reservations.len(), self.reservations.capacity())
            .map_err(|error| capacity_plan_failure(scope, error, "ply-reservation-list"))?;
        self.reservations
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| scope.allocation_failure("ply-reservation-list"))?;
        let actual = plan
            .finish(self.reservations.len(), self.reservations.capacity())
            .map_err(|error| capacity_plan_failure(scope, error, "ply-reservation-list"))?;
        debug_assert!(actual <= reserved_bytes);
        scope.resize_live(&mut reservation, actual)?;
        self.reservations.push(HeldReservation {
            reservation,
            payload: false,
        });
        Ok(())
    }
}

fn capacity_plan<T>(
    scope: &ParseScope<'_>,
    len: usize,
    capacity: usize,
    desired_capacity: usize,
    resource: &'static str,
) -> ParseResult<CapacityPlan> {
    CapacityPlan::for_type::<T>(len, capacity, desired_capacity)
        .map_err(|error| capacity_plan_failure(scope, error, resource))
}

fn capacity_plan_failure(
    scope: &ParseScope<'_>,
    error: CapacityPlanError,
    resource: &'static str,
) -> baozi_import::ParseFailure {
    match error {
        CapacityPlanError::CapacityOverflow | CapacityPlanError::AllocationTooLarge { .. } => {
            scope.allocation_failure(resource)
        }
        CapacityPlanError::InvalidState { len, capacity } => capacity_mismatch_failure(
            scope,
            resource,
            usize_to_u64(scope, capacity, resource).unwrap_or(u64::MAX),
            usize_to_u64(scope, len, resource).unwrap_or(u64::MAX),
        ),
        CapacityPlanError::StalePlan {
            expected_capacity,
            actual_capacity,
            ..
        } => capacity_mismatch_failure(
            scope,
            resource,
            usize_to_u64(scope, expected_capacity, resource).unwrap_or(u64::MAX),
            usize_to_u64(scope, actual_capacity, resource).unwrap_or(u64::MAX),
        ),
        CapacityPlanError::CapacityPlatformUnqualified {
            envelope_capacity,
            actual_capacity,
            ..
        } => capacity_mismatch_failure(
            scope,
            resource,
            usize_to_u64(scope, envelope_capacity, resource).unwrap_or(u64::MAX),
            usize_to_u64(scope, actual_capacity, resource).unwrap_or(u64::MAX),
        ),
        _ => scope.format_failure(
            crate::PlyFailure::new(
                crate::PlyFailureCode::InternalInvariant,
                "unknown capacity planning failure",
            )
            .with_context(resource),
        ),
    }
}

fn capacity_mismatch_failure(
    scope: &ParseScope<'_>,
    resource: &'static str,
    expected: u64,
    actual: u64,
) -> baozi_import::ParseFailure {
    scope.format_failure(
        crate::PlyFailure::new(
            crate::PlyFailureCode::InternalInvariant,
            "allocator capacity exceeded the admitted envelope",
        )
        .with_context(resource)
        .with_counts(expected, actual),
    )
}

pub(crate) fn decimal_digits(value: usize) -> usize {
    value
        .checked_ilog10()
        .map_or(1, |exponent| exponent as usize + 1)
}

fn ordered_map_failure(
    scope: &ParseScope<'_>,
    error: OrderedMapError,
    resource: &'static str,
) -> baozi_import::ParseFailure {
    match error {
        OrderedMapError::CapacityOverflow => scope.resource_limit_exceeded(resource, u64::MAX),
        OrderedMapError::AllocationFailed | OrderedMapError::CapacityNotReserved => {
            scope.allocation_failure(resource)
        }
        _ => scope.allocation_failure(resource),
    }
}

fn usize_to_u64(scope: &ParseScope<'_>, value: usize, resource: &'static str) -> ParseResult<u64> {
    u64::try_from(value).map_err(|_| scope.allocation_failure(resource))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_capacity_policy_qualifies_vec_string_and_ordered_map() {
        for target in [1, 4, 7, 64, 4_096] {
            assert_qualified_vec_capacity::<u8>(target);
            assert_qualified_vec_capacity::<u32>(target);
            assert_qualified_vec_capacity::<HeldReservation<'static>>(target);

            let mut value = String::new();
            let plan = CapacityPlan::for_type::<u8>(value.len(), value.capacity(), target)
                .expect("test string capacity plan must be valid");
            let admitted = plan
                .envelope_delta_bytes()
                .expect("test string envelope must be representable");
            value
                .try_reserve_exact(plan.allocator_additional())
                .expect("test string capacity must be reservable");
            let actual = plan
                .finish(value.len(), value.capacity())
                .expect("test string capacity must satisfy the shared policy");
            assert!(
                actual <= admitted,
                "String capacity policy exceeded the admitted envelope: admitted={admitted}, actual={actual}"
            );

            let mut map = OrderedMap::<String, String>::new();
            let plan = map
                .capacity_plan(target)
                .expect("test map capacity plan must be valid");
            let admitted = plan
                .envelope_delta_bytes()
                .expect("test map envelope must be representable");
            let actual = map
                .apply_capacity_plan(plan)
                .expect("test map capacity must be reservable");
            assert!(
                actual <= admitted,
                "OrderedMap capacity policy exceeded the admitted envelope: admitted={admitted}, actual={actual}"
            );
        }
    }

    fn assert_qualified_vec_capacity<T>(target: usize) {
        let mut values = Vec::<T>::new();
        let plan = CapacityPlan::for_type::<T>(values.len(), values.capacity(), target)
            .expect("test vector capacity plan must be valid");
        let admitted = plan
            .envelope_delta_bytes()
            .expect("test vector envelope must be representable");
        values
            .try_reserve_exact(plan.allocator_additional())
            .expect("test vector capacity must be reservable");
        let actual = plan
            .finish(values.len(), values.capacity())
            .expect("test vector capacity must satisfy the shared policy");
        assert!(
            actual <= admitted,
            "Vec capacity policy exceeded the admitted envelope: admitted={admitted}, actual={actual}"
        );
    }
}
