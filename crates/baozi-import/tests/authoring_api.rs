use baozi_import::SceneDraft;
use baozi_import::authoring::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, DeformationAuthoringBatch,
    GeometryAuthoringBatch, LineageAuthoringBatch, MetadataAuthoringBatch, NodeAuthoringBatch,
    Presence, ProfileAuthoringBatch, SceneAuthoringBatch, SceneSpace, SpatialAuthoringBatch,
};

#[test]
fn custom_importers_receive_one_authority_free_authoring_surface() {
    let scene_space = SceneSpace::unknown();
    let input = SceneAuthoringBatch::new(
        SpatialAuthoringBatch::new(&scene_space, &[], &[]),
        NodeAuthoringBatch::new(Presence::Present(&[]), &[]),
        AppearanceAuthoringBatch::empty(),
        GeometryAuthoringBatch::empty(),
        DeformationAuthoringBatch::empty(),
        AnimationAuthoringBatch::empty(),
        ProfileAuthoringBatch::new(
            LineageAuthoringBatch::new(&[], &[], &[], &[]),
            MetadataAuthoringBatch::empty(),
        ),
    );

    fn accepts_input(_: SceneAuthoringBatch<'_>) {}
    fn accepts_draft(_: SceneDraft) {}

    accepts_input(input);
    let _: fn(SceneDraft) = accepts_draft;
}
