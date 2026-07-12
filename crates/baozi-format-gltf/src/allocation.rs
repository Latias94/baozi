use baozi_import::{
    CapacityPlan, CapacityPlanError, ImportLiveReservation, ImportStorageClass, ParseResult,
    ParseScope, ScopedSceneBuilder,
};

const COPY_POLL_INTERVAL: usize = 4_096;

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

        let desired_capacity = values.capacity().saturating_mul(2).max(required).max(4);
        let plan = CapacityPlan::for_type::<T>(values.len(), values.capacity(), desired_capacity)
            .map_err(|error| capacity_plan_failure(scope, resource, error))?;
        let admitted = plan
            .envelope_delta_bytes()
            .map_err(|error| capacity_plan_failure(scope, resource, error))?;
        if admitted == 0 {
            plan.verify_preflight(values.len(), values.capacity())
                .map_err(|error| capacity_plan_failure(scope, resource, error))?;
            values
                .try_reserve_exact(plan.allocator_additional())
                .map_err(|_| scope.allocation_failure(resource))?;
            plan.finish(values.len(), values.capacity())
                .map_err(|error| capacity_plan_failure(scope, resource, error))?;
            scope.poll()?;
            return Ok(());
        }

        self.ensure_slots(scope, 1)?;
        let mut reservation = scope.reserve_live(self.class, admitted)?;
        plan.verify_preflight(values.len(), values.capacity())
            .map_err(|error| capacity_plan_failure(scope, resource, error))?;
        values
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| scope.allocation_failure(resource))?;
        let actual = plan
            .finish(values.len(), values.capacity())
            .map_err(|error| capacity_plan_failure(scope, resource, error))?;
        scope.resize_live(&mut reservation, actual)?;
        self.reservations.push(HeldReservation {
            reservation,
            payload: true,
        });
        scope.poll()?;
        Ok(())
    }

    pub(crate) fn reserve_string(
        &mut self,
        scope: &'scope ParseScope<'_>,
        value: &mut String,
        additional: usize,
        resource: &'static str,
    ) -> ParseResult<()> {
        let required = value
            .len()
            .checked_add(additional)
            .ok_or_else(|| scope.allocation_failure(resource))?;
        if required <= value.capacity() {
            return Ok(());
        }

        let desired_capacity = value.capacity().saturating_mul(2).max(required).max(4);
        let plan =
            CapacityPlan::with_element_size(value.len(), value.capacity(), desired_capacity, 1)
                .map_err(|error| capacity_plan_failure(scope, resource, error))?;
        let admitted = plan
            .envelope_delta_bytes()
            .map_err(|error| capacity_plan_failure(scope, resource, error))?;
        self.ensure_slots(scope, 1)?;
        let mut reservation = scope.reserve_live(self.class, admitted)?;
        plan.verify_preflight(value.len(), value.capacity())
            .map_err(|error| capacity_plan_failure(scope, resource, error))?;
        value
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| scope.allocation_failure(resource))?;
        let actual = plan
            .finish(value.len(), value.capacity())
            .map_err(|error| capacity_plan_failure(scope, resource, error))?;
        scope.resize_live(&mut reservation, actual)?;
        self.reservations.push(HeldReservation {
            reservation,
            payload: true,
        });
        scope.poll()?;
        Ok(())
    }

    pub(crate) fn copy_string(
        &mut self,
        scope: &'scope ParseScope<'_>,
        value: &str,
        resource: &'static str,
    ) -> ParseResult<String> {
        if value.is_empty() {
            return Ok(String::new());
        }
        let mut output = String::new();
        self.reserve_string(scope, &mut output, value.len(), resource)?;
        let mut next_poll = 0usize;
        for (byte_index, character) in value.char_indices() {
            if byte_index >= next_poll {
                scope.poll()?;
                next_poll = byte_index.saturating_add(COPY_POLL_INTERVAL);
            }
            output.push(character);
        }
        debug_assert_eq!(output.len(), value.len());
        Ok(output)
    }

    pub(crate) fn reserve_bytes(
        &mut self,
        scope: &'scope ParseScope<'_>,
        bytes: u64,
    ) -> ParseResult<()> {
        self.reserve_payload(scope, bytes)
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

        let minimum_capacity = self
            .reservations
            .len()
            .checked_add(required_free_slots)
            .and_then(|value| value.checked_add(2))
            .ok_or_else(|| scope.allocation_failure("gltf-reservation-list"))?;
        let desired_capacity = self
            .reservations
            .capacity()
            .saturating_mul(2)
            .max(minimum_capacity)
            .max(8);
        let plan = CapacityPlan::for_type::<HeldReservation<'_>>(
            self.reservations.len(),
            self.reservations.capacity(),
            desired_capacity,
        )
        .map_err(|error| capacity_plan_failure(scope, "gltf-reservation-list", error))?;
        let admitted = plan
            .envelope_delta_bytes()
            .map_err(|error| capacity_plan_failure(scope, "gltf-reservation-list", error))?;
        let mut reservation = scope.reserve_live(self.class, admitted)?;
        plan.verify_preflight(self.reservations.len(), self.reservations.capacity())
            .map_err(|error| capacity_plan_failure(scope, "gltf-reservation-list", error))?;
        self.reservations
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| scope.allocation_failure("gltf-reservation-list"))?;
        let actual = plan
            .finish(self.reservations.len(), self.reservations.capacity())
            .map_err(|error| capacity_plan_failure(scope, "gltf-reservation-list", error))?;
        scope.resize_live(&mut reservation, actual)?;
        self.reservations.push(HeldReservation {
            reservation,
            payload: false,
        });
        scope.poll()?;
        Ok(())
    }
}

fn capacity_plan_failure(
    scope: &ParseScope<'_>,
    resource: &'static str,
    error: CapacityPlanError,
) -> baozi_import::ParseFailure {
    match error {
        CapacityPlanError::CapacityOverflow | CapacityPlanError::AllocationTooLarge { .. } => {
            scope.allocation_failure(resource)
        }
        _ => scope.format_failure(crate::GltfFailure::resource_contract(resource, error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_capacity_policy_stays_within_the_admitted_envelope() {
        for target in [1, 4, 7, 64, 4_096] {
            assert_planned_vec_capacity::<u8>(target);
            assert_planned_vec_capacity::<u32>(target);
            assert_planned_vec_capacity::<HeldReservation<'static>>(target);

            let mut value = String::new();
            let plan = CapacityPlan::with_element_size(value.len(), value.capacity(), target, 1)
                .expect("test string capacity plan must be representable");
            value
                .try_reserve_exact(plan.allocator_additional())
                .expect("test string capacity must be reservable");
            plan.finish(value.len(), value.capacity())
                .expect("String capacity must stay within the admitted envelope");
        }
    }

    fn assert_planned_vec_capacity<T>(target: usize) {
        let mut values = Vec::<T>::new();
        let plan = CapacityPlan::for_type::<T>(values.len(), values.capacity(), target)
            .expect("test vector capacity plan must be representable");
        values
            .try_reserve_exact(plan.allocator_additional())
            .expect("test vector capacity must be reservable");
        plan.finish(values.len(), values.capacity())
            .expect("Vec capacity must stay within the admitted envelope");
    }
}
