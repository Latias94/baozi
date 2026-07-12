use baozi_import::{
    CapacityPlan, CapacityPlanError, ImportLiveReservation, ImportStorageClass, ParseFailure,
    ParseResult, ParseScope, ScopedSceneBuilder,
};

pub(crate) const CHECKPOINT_CHUNK_BYTES: usize = 4 * 1024;

fn append_string_chunked(
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

pub(crate) struct DecodedTextBuffer<'scope> {
    text: String,
    decoded_len: usize,
    _decoded: ImportLiveReservation<'scope>,
    _allocator_slack: Option<ImportLiveReservation<'scope>>,
}

impl<'scope> DecodedTextBuffer<'scope> {
    pub(crate) fn try_new(
        scope: &'scope ParseScope<'_>,
        decoded_len: usize,
        resource: &'static str,
    ) -> ParseResult<Self> {
        let plan = CapacityPlan::with_element_size(0, 0, decoded_len, 1)
            .map_err(|error| capacity_plan_failure(scope, resource, error))?;
        let envelope = plan
            .envelope_delta_bytes()
            .map_err(|error| capacity_plan_failure(scope, resource, error))?;
        let decoded_bytes =
            u64::try_from(decoded_len).map_err(|_| scope.allocation_failure(resource))?;
        let slack_bytes = envelope.checked_sub(decoded_bytes).ok_or_else(|| {
            decoded_text_contract_failure(
                scope,
                "decoded-text-envelope-below-length",
                decoded_bytes,
                envelope,
            )
        })?;

        let decoded = scope.reserve_live(ImportStorageClass::DecodedText, decoded_bytes)?;
        let mut allocator_slack = if slack_bytes == 0 {
            None
        } else {
            Some(scope.reserve_live(ImportStorageClass::BackendDecode, slack_bytes)?)
        };
        let mut text = String::new();
        plan.verify_preflight(text.len(), text.capacity())
            .map_err(|error| capacity_plan_failure(scope, resource, error))?;
        text.try_reserve_exact(plan.allocator_additional())
            .map_err(|_| scope.allocation_failure(resource))?;
        let actual = plan
            .finish(text.len(), text.capacity())
            .map_err(|error| capacity_plan_failure(scope, resource, error))?;
        let actual_slack = actual.checked_sub(decoded_bytes).ok_or_else(|| {
            decoded_text_contract_failure(
                scope,
                "decoded-text-capacity-below-length",
                decoded_bytes,
                actual,
            )
        })?;
        if actual_slack == 0 {
            drop(allocator_slack.take());
        } else if let Some(reservation) = allocator_slack.as_mut() {
            scope.resize_live(reservation, actual_slack)?;
        } else {
            return Err(decoded_text_contract_failure(
                scope,
                "decoded-text-slack-unreserved",
                0,
                actual_slack,
            ));
        }

        Ok(Self {
            text,
            decoded_len,
            _decoded: decoded,
            _allocator_slack: allocator_slack,
        })
    }

    pub(crate) fn append(
        &mut self,
        scope: &ParseScope<'_>,
        value: &str,
        resource: &'static str,
    ) -> ParseResult<()> {
        let required = self
            .text
            .len()
            .checked_add(value.len())
            .ok_or_else(|| scope.allocation_failure(resource))?;
        if required > self.decoded_len {
            return Err(decoded_text_contract_failure(
                scope,
                "decoded-text-length-exhausted",
                u64::try_from(self.decoded_len).map_err(|_| scope.allocation_failure(resource))?,
                u64::try_from(required).map_err(|_| scope.allocation_failure(resource))?,
            ));
        }
        if required > self.text.capacity() {
            return Err(decoded_text_contract_failure(
                scope,
                "decoded-text-capacity-exhausted",
                u64::try_from(self.text.capacity())
                    .map_err(|_| scope.allocation_failure(resource))?,
                u64::try_from(required).map_err(|_| scope.allocation_failure(resource))?,
            ));
        }
        self.text.push_str(value);
        Ok(())
    }

    pub(crate) fn finish(
        self,
        scope: &ParseScope<'_>,
        resource: &'static str,
    ) -> ParseResult<Self> {
        if self.text.len() != self.decoded_len {
            return Err(decoded_text_contract_failure(
                scope,
                "decoded-text-length-mismatch",
                u64::try_from(self.decoded_len).map_err(|_| scope.allocation_failure(resource))?,
                u64::try_from(self.text.len()).map_err(|_| scope.allocation_failure(resource))?,
            ));
        }
        Ok(self)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.text
    }
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
            return Ok(());
        }

        self.ensure_slots(scope, 1)?;
        let mut reservation = scope.reserve_live(self.class, admitted)?;
        plan.verify_preflight(values.len(), values.capacity())
            .map_err(|error| capacity_plan_failure(scope, resource, error))?;
        values
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| scope.allocation_failure(resource))?;
        let actual = match plan.finish(values.len(), values.capacity()) {
            Ok(actual) => actual,
            Err(error) => {
                self.hold(reservation, true);
                return Err(capacity_plan_failure(scope, resource, error));
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
        let mut output = String::new();
        self.reserve_string(scope, &mut output, value.len(), resource)?;
        append_string_chunked(scope, &mut output, value)?;
        Ok(output)
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

    fn reserve_string(
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
        let actual = match plan.finish(value.len(), value.capacity()) {
            Ok(actual) => actual,
            Err(error) => {
                self.hold(reservation, true);
                return Err(capacity_plan_failure(scope, resource, error));
            }
        };
        if let Err(failure) = scope.resize_live(&mut reservation, actual) {
            self.hold(reservation, true);
            return Err(failure);
        }
        self.hold(reservation, true);
        Ok(())
    }

    fn ensure_slots(
        &mut self,
        scope: &'scope ParseScope<'_>,
        required_free_slots: usize,
    ) -> ParseResult<()> {
        if self
            .reservations
            .capacity()
            .saturating_sub(self.reservations.len())
            >= required_free_slots
        {
            return Ok(());
        }

        let desired_capacity = self
            .reservations
            .len()
            .checked_add(required_free_slots)
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| scope.allocation_failure("stl-reservation-list"))?;
        let plan = CapacityPlan::for_type::<HeldReservation<'_>>(
            self.reservations.len(),
            self.reservations.capacity(),
            desired_capacity.max(8),
        )
        .map_err(|error| capacity_plan_failure(scope, "stl-reservation-list", error))?;
        let admitted = plan
            .envelope_delta_bytes()
            .map_err(|error| capacity_plan_failure(scope, "stl-reservation-list", error))?;
        let mut reservation = scope.reserve_live(self.class, admitted)?;
        plan.verify_preflight(self.reservations.len(), self.reservations.capacity())
            .map_err(|error| capacity_plan_failure(scope, "stl-reservation-list", error))?;
        self.reservations
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| scope.allocation_failure("stl-reservation-list"))?;
        let actual = match plan.finish(self.reservations.len(), self.reservations.capacity()) {
            Ok(actual) => actual,
            Err(error) => {
                self.hold(reservation, false);
                return Err(capacity_plan_failure(scope, "stl-reservation-list", error));
            }
        };
        if let Err(failure) = scope.resize_live(&mut reservation, actual) {
            self.hold(reservation, false);
            return Err(failure);
        }
        self.hold(reservation, false);
        Ok(())
    }

    fn hold(&mut self, reservation: ImportLiveReservation<'scope>, payload: bool) {
        self.reservations.push(HeldReservation {
            reservation,
            payload,
        });
    }
}

fn capacity_plan_failure(
    scope: &ParseScope<'_>,
    resource: &'static str,
    error: CapacityPlanError,
) -> ParseFailure {
    match error {
        CapacityPlanError::CapacityOverflow | CapacityPlanError::AllocationTooLarge { .. } => {
            scope.allocation_failure(resource)
        }
        CapacityPlanError::InvalidState { len, capacity } => capacity_contract_failure(
            scope,
            resource,
            "capacity-plan-invalid-state",
            capacity,
            len,
        ),
        CapacityPlanError::StalePlan {
            expected_capacity,
            actual_capacity,
            ..
        } => capacity_contract_failure(
            scope,
            resource,
            "capacity-plan-stale",
            expected_capacity,
            actual_capacity,
        ),
        CapacityPlanError::CapacityPlatformUnqualified {
            envelope_capacity,
            actual_capacity,
            ..
        } => capacity_contract_failure(
            scope,
            resource,
            "allocator-capacity-outside-qualified-envelope",
            envelope_capacity,
            actual_capacity,
        ),
        _ => scope.format_failure(crate::StlFailure::content(
            crate::StlFailureCode::InternalInvariant,
            resource,
            "unknown-capacity-plan-failure",
        )),
    }
}

fn capacity_contract_failure(
    scope: &ParseScope<'_>,
    resource: &'static str,
    reason: &'static str,
    expected: usize,
    actual: usize,
) -> ParseFailure {
    scope.format_failure(
        crate::StlFailure::content(crate::StlFailureCode::InternalInvariant, resource, reason)
            .with_counts(
                u64::try_from(expected).unwrap_or(u64::MAX),
                u64::try_from(actual).unwrap_or(u64::MAX),
            ),
    )
}

fn decoded_text_contract_failure(
    scope: &ParseScope<'_>,
    reason: &'static str,
    expected: u64,
    actual: u64,
) -> ParseFailure {
    scope.format_failure(
        crate::StlFailure::content(
            crate::StlFailureCode::InternalInvariant,
            "stl-decoded-text",
            reason,
        )
        .with_counts(expected, actual),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_capacity_policy_reports_actual_growth_within_the_envelope() {
        for target in [1, 4, 7, 64, 4_096] {
            assert_planned_vec_capacity::<u8>(target);
            assert_planned_vec_capacity::<u32>(target);
            assert_planned_vec_capacity::<HeldReservation<'static>>(target);

            let mut value = String::new();
            let plan = CapacityPlan::with_element_size(value.len(), value.capacity(), target, 1)
                .expect("test string capacity plan must be representable");
            let admitted = plan
                .envelope_delta_bytes()
                .expect("test string envelope must be representable");
            value
                .try_reserve_exact(plan.allocator_additional())
                .expect("test string capacity must be reservable");
            let actual = plan
                .finish(value.len(), value.capacity())
                .expect("String capacity must satisfy the shared policy");
            assert!(actual <= admitted);
        }
    }

    #[test]
    fn shared_capacity_policy_rejects_allocator_overshoot() {
        let plan = CapacityPlan::for_type::<u32>(0, 0, 3)
            .expect("test capacity plan must be representable");

        assert!(matches!(
            plan.finish(0, plan.envelope_capacity() + 1),
            Err(CapacityPlanError::CapacityPlatformUnqualified { .. })
        ));
    }

    fn assert_planned_vec_capacity<T>(target: usize) {
        let mut values = Vec::<T>::new();
        let plan = CapacityPlan::for_type::<T>(values.len(), values.capacity(), target)
            .expect("test vector capacity plan must be representable");
        let admitted = plan
            .envelope_delta_bytes()
            .expect("test vector envelope must be representable");
        values
            .try_reserve_exact(plan.allocator_additional())
            .expect("test vector capacity must be reservable");
        let actual = plan
            .finish(values.len(), values.capacity())
            .expect("Vec capacity must satisfy the shared policy");
        assert!(actual <= admitted);
    }
}
