use baozi_import::authoring::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, DeformationAuthoringBatch,
    GeometryAuthoringBatch, LineageAuthoringBatch, MetadataAuthoringBatch, NodeAuthoringBatch,
    Presence, ProfileAuthoringBatch, SceneAuthoringBatch, SceneSpace, SpatialAuthoringBatch,
};
use baozi_import::{ParseContext, ParseResult};

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
    fn submit_complete(
        context: &mut ParseContext<'_>,
        input: SceneAuthoringBatch<'_>,
    ) -> ParseResult<()> {
        context.submit_complete(input)
    }

    accepts_input(input);
    let _ = submit_complete;
}
