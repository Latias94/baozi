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

#[test]
fn scene_construction_is_nameable_only_as_one_opaque_value() {
    fn accepts_construction(
        _: baozi_core::__scene_integration::integration::SceneConstruction<'_>,
    ) {
    }
    fn accepts_error(_: baozi_core::__scene_integration::integration::SceneConstructionStartError) {
    }

    let _: fn(baozi_core::__scene_integration::integration::SceneConstruction<'_>) =
        accepts_construction;
    let _: fn(baozi_core::__scene_integration::integration::SceneConstructionStartError) =
        accepts_error;
}

#[test]
fn scene_bridge_is_available_without_feature_authority() {
    use baozi_core::__scene_integration::authoring::SceneAuthoringBatch;
    use baozi_core::__scene_integration::integration::{
        PreparedScenePublication, PreparedSceneSuccess, SceneConstruction,
        SceneConstructionStartError, ScenePublishedSuccess, SceneValidationProof,
    };

    fn accepts_proven_types(_: SceneValidationProof, _: PreparedScenePublication) {}
    let _: fn(SceneValidationProof, PreparedScenePublication) = accepts_proven_types;
    let _: fn(PreparedSceneSuccess) -> ScenePublishedSuccess = PreparedSceneSuccess::handoff;

    fn _admit_authority_free_input(input: SceneAuthoringBatch<'_>) {
        let _: Result<SceneConstruction<'_>, SceneConstructionStartError> =
            SceneConstruction::try_new(input, baozi_core::SceneValidationLimits::default());
    }
}
