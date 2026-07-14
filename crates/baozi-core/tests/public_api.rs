use baozi_core::{OrderedMap, PreparedOrderedMapCapacity};

#[test]
fn prepared_ordered_map_capacity_is_nameable_by_external_callers() {
    let mut map = OrderedMap::<String, u32>::new();
    let plan = map.capacity_plan(1).expect("one entry capacity plan");
    let prepared = map
        .prepare_capacity_replacement(plan)
        .expect("one entry replacement allocation");

    fn assert_public_type(_: &PreparedOrderedMapCapacity<'_, String, u32>) {}
    assert_public_type(&prepared);
}
