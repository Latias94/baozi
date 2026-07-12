use baozi_core::{OrderedMap, OrderedMapError};
use baozi_import::{
    CapacityPlan, CapacityPlanError, ImportLiveReservation, ImportStorageClass, ImportWorkClass,
    ParseResult, ParseScope, ScopedSceneBuilder,
};

pub(crate) const CHECKPOINT_CHUNK_BYTES: usize = 4 * 1024;

pub(crate) fn append_string_chunked(
    scope: &ParseScope<'_>,
    output: &mut String,
    value: &str,
) -> ParseResult<()> {
    let mut start = 0usize;
    while start < value.len() {
        scope.poll()?;
        let mut end = start
            .checked_add(CHECKPOINT_CHUNK_BYTES)
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

        let plan = CapacityPlan::for_type::<T>(values.len(), values.capacity(), required.max(4))
            .map_err(|error| capacity_plan_failure(scope, error, resource))?;
        let admitted = plan
            .envelope_delta_bytes()
            .map_err(|error| capacity_plan_failure(scope, error, resource))?;
        self.ensure_slots(scope, 1)?;
        let mut reservation = scope.reserve_live(self.class, admitted)?;
        plan.verify_preflight(values.len(), values.capacity())
            .map_err(|error| capacity_plan_failure(scope, error, resource))?;
        values
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| scope.allocation_failure(resource))?;
        let actual = match plan.finish(values.len(), values.capacity()) {
            Ok(actual) => actual,
            Err(error) => {
                self.hold(reservation, true);
                return Err(capacity_plan_failure(scope, error, resource));
            }
        };
        if let Err(failure) = scope.resize_live(&mut reservation, actual) {
            self.hold(reservation, true);
            return Err(failure);
        }
        self.hold(reservation, true);
        Ok(())
    }

    pub(crate) fn copy_string(
        &mut self,
        scope: &'scope ParseScope<'_>,
        value: &str,
        resource: &'static str,
    ) -> ParseResult<String> {
        let mut output = self.allocate_string(scope, value.len(), resource)?;
        append_string_chunked(scope, &mut output, value)?;
        Ok(output)
    }

    pub(crate) fn allocate_string(
        &mut self,
        scope: &'scope ParseScope<'_>,
        desired_capacity: usize,
        resource: &'static str,
    ) -> ParseResult<String> {
        let mut output = String::new();
        let plan = CapacityPlan::with_element_size(0, 0, desired_capacity, 1)
            .map_err(|error| capacity_plan_failure(scope, error, resource))?;
        let admitted = plan
            .envelope_delta_bytes()
            .map_err(|error| capacity_plan_failure(scope, error, resource))?;
        if admitted == 0 {
            return Ok(output);
        }

        self.ensure_slots(scope, 1)?;
        let mut reservation = scope.reserve_live(self.class, admitted)?;
        plan.verify_preflight(output.len(), output.capacity())
            .map_err(|error| capacity_plan_failure(scope, error, resource))?;
        output
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| scope.allocation_failure(resource))?;
        let actual = match plan.finish(output.len(), output.capacity()) {
            Ok(actual) => actual,
            Err(error) => {
                self.hold(reservation, true);
                return Err(capacity_plan_failure(scope, error, resource));
            }
        };
        if let Err(failure) = scope.resize_live(&mut reservation, actual) {
            self.hold(reservation, true);
            return Err(failure);
        }
        self.hold(reservation, true);
        Ok(output)
    }

    pub(crate) fn reserve_bytes(
        &mut self,
        scope: &'scope ParseScope<'_>,
        bytes: usize,
        resource: &'static str,
    ) -> ParseResult<()> {
        self.reserve_payload(scope, usize_to_u64(scope, bytes, resource)?)
    }

    pub(crate) fn insert_ordered_map<V>(
        &mut self,
        scope: &'scope ParseScope<'_>,
        map: &mut OrderedMap<String, V>,
        key: String,
        value: V,
        resource: &'static str,
    ) -> ParseResult<Option<V>> {
        let search_rounds = ceil_log2(
            map.len()
                .checked_add(1)
                .ok_or_else(|| scope.allocation_failure(resource))?,
        );
        let work = key
            .len()
            .checked_mul(search_rounds)
            .and_then(|work| work.checked_mul(2))
            .and_then(|work| work.checked_add(map.len()))
            .ok_or_else(|| scope.allocation_failure(resource))?;
        let work_permit = scope.begin_work(
            ImportWorkClass::WorkUnits,
            usize_to_u64(scope, work, resource)?,
        )?;
        let additional = usize::from(!map.contains_key(&key));
        if additional == 0 {
            let replaced = map
                .insert_reserved(key, value)
                .map_err(|error| ordered_map_failure(scope, error, resource))?;
            work_permit.complete()?;
            return Ok(replaced);
        }

        let plan = map
            .capacity_plan(additional)
            .map_err(|error| ordered_map_failure(scope, error, resource))?;
        let admitted = plan
            .envelope_delta_bytes()
            .map_err(|error| ordered_map_failure(scope, error, resource))?;
        if admitted == 0 {
            map.apply_capacity_plan(plan)
                .map_err(|error| ordered_map_failure(scope, error, resource))?;
            let inserted = map
                .insert_reserved(key, value)
                .map_err(|error| ordered_map_failure(scope, error, resource))?;
            work_permit.complete()?;
            return Ok(inserted);
        }
        self.ensure_slots(scope, 1)?;
        let mut reservation = scope.reserve_live(self.class, admitted)?;
        let actual = match map.apply_capacity_plan(plan) {
            Ok(actual) => actual,
            Err(error) => {
                self.hold(reservation, true);
                return Err(ordered_map_failure(scope, error, resource));
            }
        };
        if let Err(failure) = scope.resize_live(&mut reservation, actual) {
            self.hold(reservation, true);
            return Err(failure);
        }
        self.hold(reservation, true);
        let inserted = map
            .insert_reserved(key, value)
            .map_err(|error| ordered_map_failure(scope, error, resource))?;
        work_permit.complete()?;
        Ok(inserted)
    }

    pub(crate) fn adopt(self, builder: &mut ScopedSceneBuilder<'_, '_>) -> ParseResult<()> {
        let mut reservations = self.reservations;
        let mut index = 0;
        while index < reservations.len() {
            if reservations[index].payload {
                let held = reservations.swap_remove(index);
                builder.adopt_reservation(held.reservation)?;
            } else {
                index += 1;
            }
        }
        Ok(())
    }

    fn hold(&mut self, reservation: ImportLiveReservation<'scope>, payload: bool) {
        self.reservations.push(HeldReservation {
            reservation,
            payload,
        });
    }

    fn reserve_payload(&mut self, scope: &'scope ParseScope<'_>, bytes: u64) -> ParseResult<()> {
        if bytes == 0 {
            return Ok(());
        }
        self.ensure_slots(scope, 1)?;
        let reservation = scope.reserve_live(self.class, bytes)?;
        self.reservations.push(HeldReservation {
            reservation,
            payload: true,
        });
        Ok(())
    }

    fn ensure_slots(
        &mut self,
        scope: &'scope ParseScope<'_>,
        required_free_slots: usize,
    ) -> ParseResult<()> {
        if self.reservations.capacity() - self.reservations.len() >= required_free_slots {
            return Ok(());
        }

        let desired_capacity = self
            .reservations
            .len()
            .checked_add(required_free_slots)
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| scope.allocation_failure("obj-reservation-list"))?;
        let plan = CapacityPlan::for_type::<HeldReservation<'_>>(
            self.reservations.len(),
            self.reservations.capacity(),
            desired_capacity.max(8),
        )
        .map_err(|error| capacity_plan_failure(scope, error, "obj-reservation-list"))?;
        let admitted = plan
            .envelope_delta_bytes()
            .map_err(|error| capacity_plan_failure(scope, error, "obj-reservation-list"))?;
        let mut reservation = scope.reserve_live(self.class, admitted)?;
        plan.verify_preflight(self.reservations.len(), self.reservations.capacity())
            .map_err(|error| capacity_plan_failure(scope, error, "obj-reservation-list"))?;
        self.reservations
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| scope.allocation_failure("obj-reservation-list"))?;
        let actual = match plan.finish(self.reservations.len(), self.reservations.capacity()) {
            Ok(actual) => actual,
            Err(error) => {
                self.hold(reservation, false);
                return Err(capacity_plan_failure(scope, error, "obj-reservation-list"));
            }
        };
        if let Err(failure) = scope.resize_live(&mut reservation, actual) {
            self.hold(reservation, false);
            return Err(failure);
        }
        self.hold(reservation, false);
        Ok(())
    }
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
        CapacityPlanError::InvalidState { .. } => {
            scope.format_failure(crate::ObjFailure::internal("capacity-plan-invalid-state"))
        }
        CapacityPlanError::StalePlan { .. } => {
            scope.format_failure(crate::ObjFailure::internal("capacity-plan-stale"))
        }
        CapacityPlanError::CapacityPlatformUnqualified { .. } => {
            scope.format_failure(crate::ObjFailure::internal("capacity-platform-unqualified"))
        }
        _ => scope.format_failure(crate::ObjFailure::internal("unknown-capacity-plan-failure")),
    }
}

fn ordered_map_failure(
    scope: &ParseScope<'_>,
    error: OrderedMapError,
    resource: &'static str,
) -> baozi_import::ParseFailure {
    match error {
        OrderedMapError::CapacityOverflow | OrderedMapError::AllocationFailed => {
            scope.allocation_failure(resource)
        }
        OrderedMapError::CapacityNotReserved => scope.format_failure(crate::ObjFailure::internal(
            "ordered-map-insert-without-reserved-capacity",
        )),
        _ => scope.format_failure(crate::ObjFailure::internal("unknown-ordered-map-failure")),
    }
}

pub(crate) fn usize_to_u64(
    scope: &ParseScope<'_>,
    value: usize,
    resource: &'static str,
) -> ParseResult<u64> {
    u64::try_from(value).map_err(|_| scope.allocation_failure(resource))
}

fn ceil_log2(value: usize) -> usize {
    if value <= 1 {
        0
    } else {
        usize::BITS as usize - (value - 1).leading_zeros() as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_policy_rejects_allocator_overshoot() {
        let plan = CapacityPlan::for_type::<u32>(0, 0, 3).expect("valid capacity plan");
        let overshoot = plan.envelope_capacity() + 1;

        assert!(matches!(
            plan.finish(0, overshoot),
            Err(CapacityPlanError::CapacityPlatformUnqualified { .. })
        ));
    }
}
