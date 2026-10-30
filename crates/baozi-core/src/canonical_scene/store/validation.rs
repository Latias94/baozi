mod node_scene;

pub(in crate::canonical_scene::store) use super::typed_storage::animation::validation::{
    AnimationMeteredCounts, AnimationValidationScratch, AnimationValidationScratchRequirements,
};
pub(in crate::canonical_scene::store) use super::typed_storage::deformation::validation::{
    DeformationMeteredCounts, DeformationValidationScratch,
    DeformationValidationScratchRequirements,
};
pub(in crate::canonical_scene::store) use node_scene::{
    AppearanceBindingValidationScratch, AppearanceCanonicalCounts,
    AppearanceMaterialValidationScratch, AppearanceValidationScratch,
    AppearanceValidationScratchRequirements, NodeSceneValidationProof, NodeSceneValidationScratch,
    ProfileMeteredCounts, ProfileMeteredScratch, validate_node_scene_canonical_graph_metered,
};

fn require_scalar<T: Copy + Eq>(actual: Option<T>, expected: T) -> Option<()> {
    (actual == Some(expected)).then_some(())
}

fn require_optional_collection(
    presence: Option<u8>,
    count: Option<u32>,
    expected_presence: u8,
    expected_count: u32,
) -> Option<()> {
    require_scalar(presence, expected_presence)?;
    require_scalar(count, expected_count)
}
