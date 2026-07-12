use baozi_operation::{CAPACITY_POLICY_REVISION, CapacityPlan, CapacityPlanError};

#[test]
fn fresh_and_growing_vectors_stay_inside_the_power_of_two_envelope() {
    let mut fresh = Vec::<u8>::new();
    let fresh_plan = CapacityPlan::for_type::<u8>(fresh.len(), fresh.capacity(), 3)
        .expect("fresh capacity plan");
    assert_eq!(fresh_plan.envelope_capacity(), 4);
    assert_eq!(fresh_plan.envelope_delta_bytes().unwrap(), 4);
    fresh_plan
        .verify_preflight(fresh.len(), fresh.capacity())
        .unwrap();
    fresh
        .try_reserve_exact(fresh_plan.allocator_additional())
        .expect("qualified allocator should reserve the fresh envelope");
    assert_eq!(
        fresh_plan.finish(fresh.len(), fresh.capacity()).unwrap(),
        fresh.capacity() as u64
    );

    fresh.extend_from_slice(&[1, 2, 3, 4]);
    let growth_plan = CapacityPlan::for_type::<u8>(fresh.len(), fresh.capacity(), 5)
        .expect("growth capacity plan");
    assert_eq!(growth_plan.envelope_capacity(), 8);
    growth_plan
        .verify_preflight(fresh.len(), fresh.capacity())
        .unwrap();
    fresh
        .try_reserve_exact(growth_plan.allocator_additional())
        .expect("qualified allocator should reserve the growth envelope");
    assert_eq!(
        growth_plan.finish(fresh.len(), fresh.capacity()).unwrap(),
        4
    );
}

#[test]
fn plan_rejects_stale_overflow_and_unqualified_capacity() {
    let plan = CapacityPlan::for_type::<u32>(0, 0, 3).unwrap();
    assert!(matches!(
        plan.verify_preflight(1, 1),
        Err(CapacityPlanError::StalePlan { .. })
    ));
    assert!(matches!(
        plan.finish(0, 5),
        Err(CapacityPlanError::CapacityPlatformUnqualified {
            policy_revision: CAPACITY_POLICY_REVISION,
            desired_capacity: 3,
            envelope_capacity: 4,
            actual_capacity: 5,
        })
    ));
    assert_eq!(
        CapacityPlan::for_type::<u8>(0, 0, usize::MAX),
        Err(CapacityPlanError::CapacityOverflow)
    );
}

#[test]
fn zero_sized_values_need_no_allocator_or_live_byte_admission() {
    let values = Vec::<()>::new();
    let plan = CapacityPlan::for_type::<()>(values.len(), values.capacity(), 1024).unwrap();
    assert_eq!(plan.allocator_additional(), 0);
    assert_eq!(plan.envelope_delta_bytes().unwrap(), 0);
    assert_eq!(plan.finish(values.len(), values.capacity()).unwrap(), 0);
}

#[test]
fn explicit_element_size_supports_string_capacity_planning() {
    let mut value = String::new();
    let plan = CapacityPlan::with_element_size(value.len(), value.capacity(), 5, 1).unwrap();
    assert_eq!(plan.envelope_capacity(), 8);
    value
        .try_reserve_exact(plan.allocator_additional())
        .expect("qualified allocator should reserve the string envelope");
    assert_eq!(
        plan.finish(value.len(), value.capacity()).unwrap(),
        value.capacity() as u64
    );
}
