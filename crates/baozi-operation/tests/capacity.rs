use baozi_operation::{
    CAPACITY_POLICY_REVISION, CapacityPlan, CapacityPlanError, GrowableCapacityPlan,
    ReplacementCapacityPlan,
};

#[test]
fn fresh_and_growing_vectors_request_only_desired_capacity() {
    let mut fresh = Vec::<u8>::new();
    let fresh_plan = CapacityPlan::for_type::<u8>(fresh.len(), fresh.capacity(), 3)
        .expect("fresh capacity plan");
    assert_eq!(fresh_plan.envelope_capacity(), 4);
    assert_eq!(fresh_plan.envelope_delta_bytes().unwrap(), 4);
    assert_eq!(fresh_plan.allocator_additional(), 3);
    fresh_plan
        .verify_preflight(fresh.len(), fresh.capacity())
        .unwrap();
    fresh
        .try_reserve_exact(fresh_plan.allocator_additional())
        .expect("allocator should satisfy the desired fresh capacity");
    assert!(fresh.capacity() >= fresh_plan.desired_capacity());
    assert!(fresh.capacity() <= fresh_plan.envelope_capacity());
    assert_eq!(
        fresh_plan.finish(fresh.len(), fresh.capacity()).unwrap(),
        fresh.capacity() as u64
    );

    fresh.resize(fresh.capacity(), 0);
    let original_len = fresh.len();
    let original_capacity = fresh.capacity();
    let desired_capacity = original_capacity
        .checked_add(1)
        .expect("small test capacity");
    let growth_plan =
        CapacityPlan::for_type::<u8>(original_len, original_capacity, desired_capacity)
            .expect("growth capacity plan");
    assert!(growth_plan.envelope_capacity() >= desired_capacity);
    assert_eq!(growth_plan.allocator_additional(), 1);
    growth_plan
        .verify_preflight(fresh.len(), fresh.capacity())
        .unwrap();
    fresh
        .try_reserve_exact(growth_plan.allocator_additional())
        .expect("allocator should satisfy the desired growth capacity");
    assert!(fresh.capacity() >= growth_plan.desired_capacity());
    assert!(fresh.capacity() <= growth_plan.envelope_capacity());
    assert_eq!(
        growth_plan.finish(fresh.len(), fresh.capacity()).unwrap(),
        u64::try_from(fresh.capacity() - original_capacity).expect("small test capacity delta")
    );
}

#[test]
fn finish_accepts_desired_middle_and_envelope_capacities() {
    let plan = CapacityPlan::for_type::<u8>(0, 0, 5).expect("capacity plan");
    assert_eq!(plan.desired_capacity(), 5);
    assert_eq!(plan.envelope_capacity(), 8);
    assert_eq!(plan.allocator_additional(), 5);

    assert_eq!(plan.finish(0, 5).unwrap(), 5);
    assert_eq!(plan.finish(0, 6).unwrap(), 6);
    assert_eq!(plan.finish(0, 8).unwrap(), 8);
}

#[test]
fn growable_replacement_promotes_the_source_envelope_to_the_candidate_request() {
    let plan = ReplacementCapacityPlan::for_type::<u8>(3, 3, 4)
        .expect("full source requires a replacement");
    assert_eq!(plan.source().desired_capacity(), 4);
    assert_eq!(plan.source().envelope_capacity(), 4);
    assert_eq!(plan.candidate().desired_capacity(), 4);
    assert_eq!(plan.candidate().allocator_additional(), 4);
    assert_eq!(plan.candidate_envelope_bytes().unwrap(), 4);
    plan.verify_source(3, 3).unwrap();

    let wider = ReplacementCapacityPlan::for_type::<u8>(4, 4, 5)
        .expect("the next replacement must retain geometric spare capacity");
    assert_eq!(wider.source().desired_capacity(), 5);
    assert_eq!(wider.source().envelope_capacity(), 8);
    assert_eq!(wider.candidate().desired_capacity(), 8);
    assert_eq!(wider.candidate().allocator_additional(), 8);

    assert!(matches!(
        ReplacementCapacityPlan::for_type::<u8>(2, 8, 5),
        Err(CapacityPlanError::ReplacementNotRequired {
            desired_capacity: 5,
            current_capacity: 8,
        })
    ));
}

#[test]
fn growable_in_place_plan_preserves_the_requirement_and_requests_the_envelope() {
    let plan = GrowableCapacityPlan::for_type::<u8>(4, 4, 5).expect("growth plan");
    assert_eq!(plan.desired_capacity(), 5);
    assert_eq!(plan.allocator_additional(), 4);
    assert_eq!(plan.envelope_capacity(), 8);
    plan.verify_source(4, 4).unwrap();
    assert!(matches!(
        plan.verify_source(5, 8),
        Err(CapacityPlanError::StalePlan { .. })
    ));
    assert_eq!(plan.finish(4, 8).unwrap(), 4);
}

#[test]
fn growing_from_spare_capacity_requests_desired_minus_len() {
    let plan = CapacityPlan::for_type::<u8>(2, 8, 12).expect("capacity plan");
    assert_eq!(plan.envelope_capacity(), 16);
    assert_eq!(plan.allocator_additional(), 10);
    assert_eq!(plan.finish(2, 12).unwrap(), 4);
    assert_eq!(plan.finish(2, 16).unwrap(), 8);
}

#[test]
fn desired_capacity_inside_existing_spare_does_not_move_storage() {
    let mut values = Vec::with_capacity(8);
    values.extend_from_slice(&[1_u8, 2]);
    let pointer = values.as_ptr();
    let plan =
        CapacityPlan::for_type::<u8>(values.len(), values.capacity(), 5).expect("capacity plan");
    assert_eq!(plan.envelope_capacity(), 8);
    assert_eq!(plan.allocator_additional(), 3);

    values
        .try_reserve_exact(plan.allocator_additional())
        .expect("existing spare capacity satisfies the desired capacity");
    assert_eq!(values.as_ptr(), pointer);
    assert_eq!(values.capacity(), 8);
    assert_eq!(plan.finish(values.len(), values.capacity()).unwrap(), 0);
}

#[test]
fn plan_rejects_stale_overflow_and_envelope_violation() {
    let plan = CapacityPlan::for_type::<u32>(0, 0, 3).unwrap();
    assert!(matches!(
        plan.verify_preflight(1, 1),
        Err(CapacityPlanError::StalePlan { .. })
    ));
    assert!(matches!(
        plan.finish(0, 5),
        Err(CapacityPlanError::CapacityEnvelopeViolated {
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
        .expect("allocator should satisfy the desired string capacity");
    assert_eq!(
        plan.finish(value.len(), value.capacity()).unwrap(),
        value.capacity() as u64
    );
}
