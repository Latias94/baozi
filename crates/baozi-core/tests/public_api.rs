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

#[cfg(feature = "_scene-integration")]
#[test]
fn scene_construction_is_nameable_only_as_one_opaque_value() {
    fn accepts_construction(_: baozi_core::scene::integration::SceneConstruction<'_>) {}
    fn accepts_error(_: baozi_core::scene::integration::SceneConstructionStartError) {}

    let _: fn(baozi_core::scene::integration::SceneConstruction<'_>) = accepts_construction;
    let _: fn(baozi_core::scene::integration::SceneConstructionStartError) = accepts_error;
}
