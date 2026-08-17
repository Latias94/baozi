#![allow(
    clippy::result_large_err,
    reason = "appearance authoring failures retain bounded control evidence inline without a failure-path allocation"
)]

use std::error::Error;
use std::fmt;

use baozi_operation::{CapacityPlan, CapacityPlanError};
use thiserror::Error;

use crate::appearance::{
    AppearanceBehaviorKind, AppearanceBuildError, MaterialSemanticsRef,
    MaterialTextureBindingSemanticsValidator, TextureBindingSemanticsRef, TextureRole,
    validate_material_semantics, validate_texture_binding_semantics,
};
use crate::entity::validate_entity_name_str;
use crate::image::{MAX_ENCODED_IMAGE_BYTES, SamplerState};
use crate::{SCENE_VALIDATION_POLL_INTERVAL, SceneValidationLimits, SceneValidationWork};

use super::super::super::schema::appearance::{
    APPEARANCE_BINDING_SEMANTIC_STRIDE, APPEARANCE_CHANNEL_ZERO, APPEARANCE_MATERIAL_FACTOR_STRIDE,
    APPEARANCE_PRESENCE_ABSENT, APPEARANCE_PRESENCE_PRESENT, encode_appearance_alpha_mode,
    encode_appearance_blend_operation, encode_appearance_channel,
    encode_appearance_color_interpretation, encode_appearance_coordinate_source,
    encode_appearance_shading_model, encode_appearance_texture_role,
};
use super::super::control::CanonicalAuthoringControl;
use super::super::input::appearance::{
    AppearanceAuthoringBatch, ImageAuthoring, ImageLocalId, ImagePayloadAuthoring,
    MaterialAuthoring, MaterialBindingAuthoring, SamplerAuthoring, SamplerLocalId,
    TextureAuthoring, TextureLocalId,
};
use super::super::input::{Presence, ResourceSnapshotAuthoringTable};

const LOGICAL_U8_BYTES: u64 = 1;
const LOGICAL_U32_BYTES: u64 = 4;
const LOGICAL_F32_BYTES: u64 = 4;
const LOGICAL_REFERENCE_BYTES: u64 = 4;

const APPEARANCE_DIRECTORY_FIXED_BYTES: u64 = 12 * LOGICAL_U32_BYTES;
const IMAGE_ROW_FIXED_BYTES: u64 = (5 * LOGICAL_U32_BYTES) + (3 * LOGICAL_U8_BYTES);
const SAMPLER_ROW_BYTES: u64 = LOGICAL_U32_BYTES + (6 * LOGICAL_U8_BYTES);
const TEXTURE_ROW_BYTES: u64 = (3 * LOGICAL_U32_BYTES) + (2 * LOGICAL_REFERENCE_BYTES);
const MATERIAL_ROW_FIXED_BYTES: u64 =
    (7 * LOGICAL_U32_BYTES) + LOGICAL_F32_BYTES + (7 * LOGICAL_U8_BYTES);
const BINDING_ROW_FIXED_BYTES: u64 =
    (7 * LOGICAL_U32_BYTES) + LOGICAL_REFERENCE_BYTES + (9 * LOGICAL_U8_BYTES);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum AppearanceAuthoringLimit {
    Entities,
    Edges,
    References,
    SemanticCells,
    CanonicalBytes,
    Work,
    ScratchBytes,
}

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum AppearanceAuthoringSemanticError {
    #[error("appearance authoring count for {collection} exceeds the u32 canonical domain")]
    CountDomainTooLarge { collection: &'static str },
    #[error("{limit:?} value {actual} exceeds limit {maximum}")]
    LimitExceeded {
        limit: AppearanceAuthoringLimit,
        actual: u64,
        maximum: u64,
    },
    #[error("appearance authoring accounting overflowed for {dimension}")]
    AccountingOverflow { dimension: &'static str },
    #[error("image {image} has an empty encoded payload")]
    EmptyEncodedPayload { image: usize },
    #[error(
        "image {image} encoded payload has {actual} bytes; the canonical per-image maximum is {maximum}"
    )]
    EncodedPayloadTooLarge {
        image: usize,
        actual: usize,
        maximum: usize,
    },
    #[error(
        "session resource snapshot table has {snapshot_count} rows; the profile declares {resource_count} resources"
    )]
    ResourceSnapshotCardinalityMismatch {
        snapshot_count: usize,
        resource_count: usize,
    },
    #[error(
        "image {image} references session resource {resource}; the profile declares {resource_count} resources"
    )]
    ImageResourceOutOfRange {
        image: usize,
        resource: u32,
        resource_count: usize,
    },
    #[error("image {image} session resource {resource} has no snapshotted bytes")]
    ImageResourceSnapshotAbsent { image: usize, resource: u32 },
    #[error(
        "material {material} binding range starts at {actual}; canonical owner coverage expected {expected}"
    )]
    MaterialBindingRangeOrder {
        material: usize,
        expected: u32,
        actual: u32,
    },
    #[error("material {material} binding range exceeds binding count {binding_count}")]
    MaterialBindingRangeOutOfBounds {
        material: usize,
        binding_count: usize,
    },
    #[error("flat binding table has {unowned} rows without exactly one material owner")]
    UnownedBindings { unowned: usize },
    #[error("texture {texture} image {image} exceeds image count {image_count}")]
    TextureImageOutOfRange {
        texture: usize,
        image: u32,
        image_count: u32,
    },
    #[error("texture {texture} sampler {sampler} exceeds sampler count {sampler_count}")]
    TextureSamplerOutOfRange {
        texture: usize,
        sampler: u32,
        sampler_count: u32,
    },
    #[error("binding {binding} texture {texture} exceeds texture count {texture_count}")]
    BindingTextureOutOfRange {
        binding: usize,
        texture: u32,
        texture_count: u32,
    },
    #[error(
        "binding {binding} uses a namespaced texture role before extension authority is retained by the production appearance rail"
    )]
    NamespacedTextureRoleUnsupported { binding: usize },
    #[error(
        "binding {binding} uses namespaced {behavior:?} before extension authority is retained by the production appearance rail"
    )]
    NamespacedBindingBehaviorUnsupported {
        binding: usize,
        behavior: AppearanceBehaviorKind,
    },
    #[error("binding {binding} has invalid canonical usage semantics")]
    InvalidBindingSemantics {
        binding: usize,
        #[source]
        source: AppearanceBuildError,
    },
    #[error("binding {binding} passed semantic validation but could not enter canonical encoding")]
    BindingEncodingInvariant { binding: usize },
    #[error("material {material} has invalid canonical material semantics")]
    InvalidMaterialSemantics {
        material: usize,
        #[source]
        source: AppearanceBuildError,
    },
    #[error("material {material} has an invalid entity name")]
    InvalidMaterialName {
        material: usize,
        #[source]
        source: crate::entity::EntityNameError,
    },
    #[error(
        "material {material} passed semantic validation but could not enter canonical encoding"
    )]
    MaterialEncodingInvariant { material: usize },
    #[error(
        "material {material} uses namespaced {behavior:?} before extension authority is retained by the production appearance rail"
    )]
    NamespacedMaterialBehaviorUnsupported {
        material: usize,
        behavior: AppearanceBehaviorKind,
    },
    #[error("material {material} has conflicting canonical binding authority")]
    InvalidMaterialBindingSemantics {
        material: usize,
        #[source]
        source: AppearanceBuildError,
    },
    #[error("scratch capacity planning failed for {resource}: {cause}")]
    ScratchCapacity {
        resource: &'static str,
        #[source]
        cause: CapacityPlanError,
    },
    #[error("scratch allocation failed for {resource}")]
    ScratchAllocation { resource: &'static str },
}

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum AppearanceAuthoringFailure<E>
where
    E: Error + Send + Sync + 'static,
{
    #[error(transparent)]
    Semantic(#[from] AppearanceAuthoringSemanticError),
    #[error("appearance authoring validation control failed")]
    Control(#[source] E),
}

struct ValidatedAppearanceImagePlan<'input> {
    input_index: usize,
    payload: &'input [u8],
    payload_offset: u32,
    payload_count: u32,
    media_type_offset: u32,
    media_type_count: u32,
    media_type_present: bool,
}

struct ValidatedAppearanceMaterialPlan {
    input_index: usize,
    binding_start: usize,
    binding_end: usize,
    shading_model_tag: u8,
    alpha_mode_tag: u8,
    alpha_cutoff: f32,
    double_sided_tag: u8,
    pbr_metallic_roughness_presence: u8,
    pbr_specular_glossiness_presence: u8,
    legacy_presence: u8,
    factor_offset: u32,
    name_offset: u32,
    name_count: u32,
}

struct ValidatedAppearanceBindingPlan {
    input_index: usize,
    role_tag: u8,
    coordinate_source_tag: u8,
    coordinate_set: u32,
    channel_lane_count: u8,
    channel_tags: [u8; 4],
    color_interpretation_tag: u8,
    semantic_scalar_offset: u32,
    blend_operation_tag: u8,
    source_key_offset: u32,
    source_key_count: u32,
}

pub(in crate::canonical_scene::store) struct ValidatedAppearanceImage<'view, 'input> {
    input: &'view ImageAuthoring<'input>,
    plan: &'view ValidatedAppearanceImagePlan<'input>,
}

impl<'input> ValidatedAppearanceImage<'_, 'input> {
    pub(in crate::canonical_scene::store) const fn input_index(&self) -> usize {
        self.plan.input_index
    }

    pub(in crate::canonical_scene::store) const fn payload_offset(&self) -> u32 {
        self.plan.payload_offset
    }

    pub(in crate::canonical_scene::store) const fn payload_count(&self) -> u32 {
        self.plan.payload_count
    }

    pub(in crate::canonical_scene::store) const fn media_type_offset(&self) -> u32 {
        self.plan.media_type_offset
    }

    pub(in crate::canonical_scene::store) const fn media_type_count(&self) -> u32 {
        self.plan.media_type_count
    }

    pub(in crate::canonical_scene::store) const fn payload(&self) -> &'input [u8] {
        self.plan.payload
    }

    pub(in crate::canonical_scene::store) const fn media_type(
        &self,
    ) -> Presence<&'input crate::extension::ExtensionMediaType> {
        match self.input.payload() {
            ImagePayloadAuthoring::Encoded { media_type, .. }
            | ImagePayloadAuthoring::SessionResource { media_type, .. } => media_type,
        }
    }

    pub(in crate::canonical_scene::store) const fn media_type_present(&self) -> bool {
        self.plan.media_type_present
    }
}

pub(in crate::canonical_scene::store) struct ValidatedAppearanceSampler<'view> {
    input: &'view SamplerAuthoring,
}

impl ValidatedAppearanceSampler<'_> {
    pub(in crate::canonical_scene::store) const fn state(&self) -> SamplerState {
        self.input.state()
    }
}

pub(in crate::canonical_scene::store) struct ValidatedAppearanceTexture<'view> {
    input: &'view TextureAuthoring,
}

impl ValidatedAppearanceTexture<'_> {
    pub(in crate::canonical_scene::store) const fn image(&self) -> ImageLocalId {
        self.input.image()
    }

    pub(in crate::canonical_scene::store) const fn sampler(&self) -> SamplerLocalId {
        self.input.sampler()
    }
}

pub(in crate::canonical_scene::store) struct ValidatedAppearanceBinding<'view, 'input> {
    input: &'view MaterialBindingAuthoring<'input>,
    plan: &'view ValidatedAppearanceBindingPlan,
}

impl<'input> ValidatedAppearanceBinding<'_, 'input> {
    pub(in crate::canonical_scene::store) const fn input_index(&self) -> usize {
        self.plan.input_index
    }

    pub(in crate::canonical_scene::store) const fn texture(&self) -> TextureLocalId {
        self.input.texture()
    }

    pub(in crate::canonical_scene::store) const fn role(&self) -> &'input TextureRole {
        self.input.role()
    }

    pub(in crate::canonical_scene::store) const fn role_tag(&self) -> u8 {
        self.plan.role_tag
    }

    pub(in crate::canonical_scene::store) const fn semantics(
        &self,
    ) -> TextureBindingSemanticsRef<'input> {
        self.input.semantics()
    }

    pub(in crate::canonical_scene::store) const fn coordinate_source_tag(&self) -> u8 {
        self.plan.coordinate_source_tag
    }

    pub(in crate::canonical_scene::store) const fn coordinate_set(&self) -> u32 {
        self.plan.coordinate_set
    }

    pub(in crate::canonical_scene::store) const fn channel_lane_count(&self) -> u8 {
        self.plan.channel_lane_count
    }

    pub(in crate::canonical_scene::store) const fn channel_tags(&self) -> [u8; 4] {
        self.plan.channel_tags
    }

    pub(in crate::canonical_scene::store) const fn color_interpretation_tag(&self) -> u8 {
        self.plan.color_interpretation_tag
    }

    pub(in crate::canonical_scene::store) const fn semantic_scalar_offset(&self) -> u32 {
        self.plan.semantic_scalar_offset
    }

    pub(in crate::canonical_scene::store) const fn blend_operation_tag(&self) -> u8 {
        self.plan.blend_operation_tag
    }

    pub(in crate::canonical_scene::store) const fn source_key_offset(&self) -> u32 {
        self.plan.source_key_offset
    }

    pub(in crate::canonical_scene::store) const fn source_key_count(&self) -> u32 {
        self.plan.source_key_count
    }
}

pub(in crate::canonical_scene::store) struct ValidatedAppearanceMaterial<'view, 'input> {
    input: &'view MaterialAuthoring<'input>,
    plan: &'view ValidatedAppearanceMaterialPlan,
    bindings: &'view [MaterialBindingAuthoring<'input>],
    binding_plans: &'view [ValidatedAppearanceBindingPlan],
}

impl<'view, 'input> ValidatedAppearanceMaterial<'view, 'input> {
    pub(in crate::canonical_scene::store) const fn input_index(&self) -> usize {
        self.plan.input_index
    }

    pub(in crate::canonical_scene::store) const fn authored(&self) -> MaterialAuthoring<'input> {
        *self.input
    }

    pub(in crate::canonical_scene::store) const fn semantics(
        &self,
    ) -> MaterialSemanticsRef<'input> {
        self.input.semantics()
    }

    pub(in crate::canonical_scene::store) const fn shading_model_tag(&self) -> u8 {
        self.plan.shading_model_tag
    }

    pub(in crate::canonical_scene::store) const fn alpha_mode_tag(&self) -> u8 {
        self.plan.alpha_mode_tag
    }

    pub(in crate::canonical_scene::store) const fn alpha_cutoff(&self) -> f32 {
        self.plan.alpha_cutoff
    }

    pub(in crate::canonical_scene::store) const fn double_sided_tag(&self) -> u8 {
        self.plan.double_sided_tag
    }

    pub(in crate::canonical_scene::store) const fn pbr_metallic_roughness_presence(&self) -> u8 {
        self.plan.pbr_metallic_roughness_presence
    }

    pub(in crate::canonical_scene::store) const fn pbr_specular_glossiness_presence(&self) -> u8 {
        self.plan.pbr_specular_glossiness_presence
    }

    pub(in crate::canonical_scene::store) const fn legacy_presence(&self) -> u8 {
        self.plan.legacy_presence
    }

    pub(in crate::canonical_scene::store) const fn factor_offset(&self) -> u32 {
        self.plan.factor_offset
    }

    pub(in crate::canonical_scene::store) const fn name_offset(&self) -> u32 {
        self.plan.name_offset
    }

    pub(in crate::canonical_scene::store) const fn name_count(&self) -> u32 {
        self.plan.name_count
    }

    pub(in crate::canonical_scene::store) fn bindings(
        &self,
    ) -> impl ExactSizeIterator<Item = ValidatedAppearanceBinding<'_, 'input>> + '_ {
        self.bindings
            .iter()
            .zip(self.binding_plans)
            .map(|(input, plan)| ValidatedAppearanceBinding { input, plan })
    }
}

#[must_use = "dropping the validated appearance batch releases its compiled-plan reservation"]
pub(in crate::canonical_scene::store) struct ValidatedAppearanceBatch<'input, R> {
    compiled: AppearanceValidationStorage<'input, R>,
    input: AppearanceAuthoringBatch<'input>,
}

struct AppearanceValidationStorage<'input, R> {
    image_count: u32,
    sampler_count: u32,
    texture_count: u32,
    material_count: u32,
    binding_count: u32,
    image_payload_byte_count: u32,
    media_type_byte_count: u32,
    source_key_byte_count: u32,
    material_name_byte_count: u32,
    binding_semantic_scalar_count: u32,
    material_factor_scalar_count: u32,
    payload_block_count: u32,
    entity_count: u64,
    edge_count: u64,
    reference_count: u64,
    semantic_cell_count: u64,
    canonical_bytes: u64,
    validation_work: u64,
    scratch_bytes: u64,
    image_plans: Vec<ValidatedAppearanceImagePlan<'input>>,
    material_plans: Vec<ValidatedAppearanceMaterialPlan>,
    binding_plans: Vec<ValidatedAppearanceBindingPlan>,
    #[cfg(test)]
    storage_drop_probe: Option<AppearanceStorageDropProbe>,
    // Every compiled-plan allocation above must drop before its accounting token.
    reservation: R,
}

impl<'input, R> ValidatedAppearanceBatch<'input, R> {
    const fn new(
        input: AppearanceAuthoringBatch<'input>,
        compiled: AppearanceValidationStorage<'input, R>,
    ) -> Self {
        Self { compiled, input }
    }

    pub(in crate::canonical_scene::store) const fn input(
        &self,
    ) -> AppearanceAuthoringBatch<'input> {
        self.input
    }

    pub(in crate::canonical_scene::store) const fn image_count(&self) -> u32 {
        self.compiled.image_count
    }

    pub(in crate::canonical_scene::store) const fn sampler_count(&self) -> u32 {
        self.compiled.sampler_count
    }

    pub(in crate::canonical_scene::store) const fn texture_count(&self) -> u32 {
        self.compiled.texture_count
    }

    pub(in crate::canonical_scene::store) const fn material_count(&self) -> u32 {
        self.compiled.material_count
    }

    pub(in crate::canonical_scene::store) const fn binding_count(&self) -> u32 {
        self.compiled.binding_count
    }

    pub(in crate::canonical_scene::store) const fn image_payload_byte_count(&self) -> u32 {
        self.compiled.image_payload_byte_count
    }

    pub(in crate::canonical_scene::store) const fn media_type_byte_count(&self) -> u32 {
        self.compiled.media_type_byte_count
    }

    pub(in crate::canonical_scene::store) const fn source_key_byte_count(&self) -> u32 {
        self.compiled.source_key_byte_count
    }

    pub(in crate::canonical_scene::store) const fn material_name_byte_count(&self) -> u32 {
        self.compiled.material_name_byte_count
    }

    pub(in crate::canonical_scene::store) const fn binding_semantic_scalar_count(&self) -> u32 {
        self.compiled.binding_semantic_scalar_count
    }

    pub(in crate::canonical_scene::store) const fn material_factor_scalar_count(&self) -> u32 {
        self.compiled.material_factor_scalar_count
    }

    pub(in crate::canonical_scene::store) const fn payload_block_count(&self) -> u32 {
        self.compiled.payload_block_count
    }

    pub(in crate::canonical_scene::store) const fn entity_count(&self) -> u64 {
        self.compiled.entity_count
    }

    pub(in crate::canonical_scene::store) const fn edge_count(&self) -> u64 {
        self.compiled.edge_count
    }

    pub(in crate::canonical_scene::store) const fn reference_count(&self) -> u64 {
        self.compiled.reference_count
    }

    pub(in crate::canonical_scene::store) const fn semantic_cell_count(&self) -> u64 {
        self.compiled.semantic_cell_count
    }

    pub(in crate::canonical_scene::store) const fn canonical_bytes(&self) -> u64 {
        self.compiled.canonical_bytes
    }

    pub(in crate::canonical_scene::store) const fn validation_work(&self) -> u64 {
        self.compiled.validation_work
    }

    pub(in crate::canonical_scene::store) const fn scratch_bytes(&self) -> u64 {
        self.compiled.scratch_bytes
    }

    pub(in crate::canonical_scene::store) fn images(
        &self,
    ) -> impl ExactSizeIterator<Item = ValidatedAppearanceImage<'_, 'input>> + '_ {
        self.input
            .images()
            .iter()
            .zip(&self.compiled.image_plans)
            .map(|(input, plan)| ValidatedAppearanceImage { input, plan })
    }

    pub(in crate::canonical_scene::store) fn image(
        &self,
        index: usize,
    ) -> Option<ValidatedAppearanceImage<'_, 'input>> {
        let input = self.input.images().get(index)?;
        let plan = self.compiled.image_plans.get(index)?;
        Some(ValidatedAppearanceImage { input, plan })
    }

    pub(in crate::canonical_scene::store) fn samplers(
        &self,
    ) -> impl ExactSizeIterator<Item = ValidatedAppearanceSampler<'_>> + '_ {
        self.input
            .samplers()
            .iter()
            .map(|input| ValidatedAppearanceSampler { input })
    }

    pub(in crate::canonical_scene::store) fn textures(
        &self,
    ) -> impl ExactSizeIterator<Item = ValidatedAppearanceTexture<'_>> + '_ {
        self.input
            .textures()
            .iter()
            .map(|input| ValidatedAppearanceTexture { input })
    }

    pub(in crate::canonical_scene::store) fn bindings(
        &self,
    ) -> impl ExactSizeIterator<Item = ValidatedAppearanceBinding<'_, 'input>> + '_ {
        self.input
            .bindings()
            .iter()
            .zip(&self.compiled.binding_plans)
            .map(|(input, plan)| ValidatedAppearanceBinding { input, plan })
    }

    pub(in crate::canonical_scene::store) fn materials(
        &self,
    ) -> impl ExactSizeIterator<Item = ValidatedAppearanceMaterial<'_, 'input>> + '_ {
        self.input
            .materials()
            .iter()
            .zip(&self.compiled.material_plans)
            .map(|(input, plan)| ValidatedAppearanceMaterial {
                input,
                plan,
                bindings: &self.input.bindings()[plan.binding_start..plan.binding_end],
                binding_plans: &self.compiled.binding_plans[plan.binding_start..plan.binding_end],
            })
    }

    pub(in crate::canonical_scene::store) fn material(
        &self,
        index: usize,
    ) -> Option<ValidatedAppearanceMaterial<'_, 'input>> {
        let input = self.input.materials().get(index)?;
        let plan = self.compiled.material_plans.get(index)?;
        let bindings = self
            .input
            .bindings()
            .get(plan.binding_start..plan.binding_end)?;
        let binding_plans = self
            .compiled
            .binding_plans
            .get(plan.binding_start..plan.binding_end)?;
        Some(ValidatedAppearanceMaterial {
            input,
            plan,
            bindings,
            binding_plans,
        })
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn install_storage_drop_probe(
        &mut self,
        stage: std::rc::Rc<std::cell::Cell<u8>>,
    ) {
        self.compiled.storage_drop_probe = Some(AppearanceStorageDropProbe(stage));
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store::production) fn into_compiled_facts_for_test(
        self,
    ) -> (u32, usize, usize, usize, R) {
        let AppearanceValidationStorage {
            image_count,
            image_plans,
            material_plans,
            binding_plans,
            reservation,
            ..
        } = self.compiled;
        (
            image_count,
            image_plans.len(),
            material_plans.len(),
            binding_plans.len(),
            reservation,
        )
    }
}

#[cfg(test)]
struct AppearanceStorageDropProbe(std::rc::Rc<std::cell::Cell<u8>>);

#[cfg(test)]
impl Drop for AppearanceStorageDropProbe {
    fn drop(&mut self) {
        assert_eq!(
            self.0.replace(1),
            0,
            "compiled appearance storage must begin the coupled drop sequence"
        );
    }
}

impl<R> fmt::Debug for ValidatedAppearanceBatch<'_, R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedAppearanceBatch")
            .field("image_count", &self.compiled.image_count)
            .field("sampler_count", &self.compiled.sampler_count)
            .field("texture_count", &self.compiled.texture_count)
            .field("material_count", &self.compiled.material_count)
            .field("binding_count", &self.compiled.binding_count)
            .field(
                "image_payload_byte_count",
                &self.compiled.image_payload_byte_count,
            )
            .field(
                "media_type_byte_count",
                &self.compiled.media_type_byte_count,
            )
            .field(
                "source_key_byte_count",
                &self.compiled.source_key_byte_count,
            )
            .field(
                "material_name_byte_count",
                &self.compiled.material_name_byte_count,
            )
            .field(
                "binding_semantic_scalar_count",
                &self.compiled.binding_semantic_scalar_count,
            )
            .field(
                "material_factor_scalar_count",
                &self.compiled.material_factor_scalar_count,
            )
            .field("payload_block_count", &self.compiled.payload_block_count)
            .field("entity_count", &self.compiled.entity_count)
            .field("edge_count", &self.compiled.edge_count)
            .field("reference_count", &self.compiled.reference_count)
            .field("semantic_cell_count", &self.compiled.semantic_cell_count)
            .field("canonical_bytes", &self.compiled.canonical_bytes)
            .field("validation_work", &self.compiled.validation_work)
            .field("scratch_bytes", &self.compiled.scratch_bytes)
            .field("reservation", &"<held>")
            .finish()
    }
}

struct AppearanceValidationScratch<'input, R> {
    image_plans: Vec<ValidatedAppearanceImagePlan<'input>>,
    material_plans: Vec<ValidatedAppearanceMaterialPlan>,
    binding_plans: Vec<ValidatedAppearanceBindingPlan>,
    // Every allocation above must drop before its accounting token.
    reservation: R,
}

type AppearanceValidationResult<T, C> =
    Result<T, AppearanceAuthoringFailure<<C as CanonicalAuthoringControl>::Error>>;

struct AppearanceValidationContext<'control, C: CanonicalAuthoringControl> {
    control: &'control mut C,
    limits: SceneValidationLimits,
    work: u64,
    work_since_poll: u64,
    edges: u64,
    references: u64,
    semantic_cells: u64,
    canonical_bytes: u64,
}

impl<'control, C: CanonicalAuthoringControl> AppearanceValidationContext<'control, C> {
    const fn new(limits: SceneValidationLimits, control: &'control mut C) -> Self {
        Self {
            control,
            limits,
            work: 0,
            work_since_poll: 0,
            edges: 0,
            references: 0,
            semantic_cells: 0,
            canonical_bytes: 0,
        }
    }

    fn poll_now(&mut self) -> AppearanceValidationResult<(), C> {
        self.control
            .poll()
            .map_err(AppearanceAuthoringFailure::Control)?;
        self.work_since_poll = 0;
        Ok(())
    }

    fn finish(&mut self) -> AppearanceValidationResult<(), C> {
        self.poll_now()
    }

    fn run_unit<R>(
        &mut self,
        kind: SceneValidationWork,
        action: impl FnMut() -> R,
    ) -> AppearanceValidationResult<R, C> {
        let final_work = self.work.checked_add(1).ok_or(
            AppearanceAuthoringSemanticError::AccountingOverflow {
                dimension: "validation work",
            },
        )?;
        enforce_limit::<C::Error>(
            AppearanceAuthoringLimit::Work,
            final_work,
            self.limits.max_work(),
        )?;
        let result = self
            .control
            .run_unit(kind, action)
            .map_err(AppearanceAuthoringFailure::Control)?;
        self.work = final_work;
        self.work_since_poll = self.work_since_poll.checked_add(1).ok_or(
            AppearanceAuthoringSemanticError::AccountingOverflow {
                dimension: "validation work poll interval",
            },
        )?;
        if self.work_since_poll == SCENE_VALIDATION_POLL_INTERVAL {
            self.poll_now()?;
        }
        Ok(result)
    }

    fn add_relations(&mut self, amount: u64) -> AppearanceValidationResult<(), C> {
        for _ in 0..amount {
            let current_edges = self.edges;
            let max_edges = self.limits.max_edges();
            let final_edges = self.run_unit(SceneValidationWork::GraphEdge, || {
                let final_edges = current_edges.checked_add(1).ok_or(
                    AppearanceAuthoringSemanticError::AccountingOverflow { dimension: "edges" },
                )?;
                if final_edges > max_edges {
                    return Err(AppearanceAuthoringSemanticError::LimitExceeded {
                        limit: AppearanceAuthoringLimit::Edges,
                        actual: final_edges,
                        maximum: max_edges,
                    });
                }
                Ok::<u64, AppearanceAuthoringSemanticError>(final_edges)
            })??;
            self.edges = final_edges;

            let current_references = self.references;
            let max_references = self.limits.max_references();
            let final_references = self.run_unit(SceneValidationWork::Reference, || {
                let final_references = current_references.checked_add(1).ok_or(
                    AppearanceAuthoringSemanticError::AccountingOverflow {
                        dimension: "references",
                    },
                )?;
                if final_references > max_references {
                    return Err(AppearanceAuthoringSemanticError::LimitExceeded {
                        limit: AppearanceAuthoringLimit::References,
                        actual: final_references,
                        maximum: max_references,
                    });
                }
                Ok::<u64, AppearanceAuthoringSemanticError>(final_references)
            })??;
            self.references = final_references;
        }
        Ok(())
    }

    fn add_payload_bytes(&mut self, amount: u64) -> AppearanceValidationResult<(), C> {
        self.add_payload_cells_and_bytes(amount, amount)
    }

    fn add_payload_cells_and_bytes(
        &mut self,
        cells: u64,
        bytes: u64,
    ) -> AppearanceValidationResult<(), C> {
        let final_cells = self.semantic_cells.checked_add(cells).ok_or(
            AppearanceAuthoringSemanticError::AccountingOverflow {
                dimension: "semantic cells",
            },
        )?;
        let final_bytes = self.canonical_bytes.checked_add(bytes).ok_or(
            AppearanceAuthoringSemanticError::AccountingOverflow {
                dimension: "canonical appearance logical footprint",
            },
        )?;
        enforce_limit::<C::Error>(
            AppearanceAuthoringLimit::SemanticCells,
            final_cells,
            self.limits.max_payload_cells(),
        )?;
        enforce_limit::<C::Error>(
            AppearanceAuthoringLimit::CanonicalBytes,
            final_bytes,
            self.limits.max_canonical_bytes(),
        )?;
        self.semantic_cells = final_cells;
        self.canonical_bytes = final_bytes;
        Ok(())
    }

    fn set_structural_bytes(&mut self, bytes: u64) -> AppearanceValidationResult<(), C> {
        enforce_limit::<C::Error>(
            AppearanceAuthoringLimit::CanonicalBytes,
            bytes,
            self.limits.max_canonical_bytes(),
        )?;
        self.canonical_bytes = bytes;
        Ok(())
    }

    fn scratch_plan<T>(
        &self,
        count: usize,
        resource: &'static str,
    ) -> Result<CapacityPlan, AppearanceAuthoringSemanticError> {
        CapacityPlan::for_type::<T>(0, 0, count)
            .map_err(|cause| AppearanceAuthoringSemanticError::ScratchCapacity { resource, cause })
    }

    fn reserve_scratch(&mut self, bytes: u64) -> AppearanceValidationResult<C::Reservation, C> {
        enforce_limit::<C::Error>(
            AppearanceAuthoringLimit::ScratchBytes,
            bytes,
            self.limits.max_scratch_bytes(),
        )?;
        let reservation = self
            .control
            .reserve_scratch(bytes)
            .map_err(AppearanceAuthoringFailure::Control)?;
        self.control
            .validate_scratch(&reservation)
            .map_err(AppearanceAuthoringFailure::Control)?;
        Ok(reservation)
    }

    fn resize_scratch(
        &mut self,
        reservation: &mut C::Reservation,
        bytes: u64,
    ) -> AppearanceValidationResult<(), C> {
        enforce_limit::<C::Error>(
            AppearanceAuthoringLimit::ScratchBytes,
            bytes,
            self.limits.max_scratch_bytes(),
        )?;
        self.control
            .resize_scratch(reservation, bytes)
            .map_err(AppearanceAuthoringFailure::Control)
    }

    fn allocate_scratch<T>(
        &self,
        plan: CapacityPlan,
        resource: &'static str,
    ) -> Result<(Vec<T>, u64), AppearanceAuthoringSemanticError> {
        plan.verify_preflight(0, 0).map_err(|cause| {
            AppearanceAuthoringSemanticError::ScratchCapacity { resource, cause }
        })?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| AppearanceAuthoringSemanticError::ScratchAllocation { resource })?;
        let actual_bytes = plan
            .finish(values.len(), values.capacity())
            .map_err(|cause| AppearanceAuthoringSemanticError::ScratchCapacity {
                resource,
                cause,
            })?;
        Ok((values, actual_bytes))
    }
}

pub(in crate::canonical_scene::store) fn appearance_shape_scratch_envelope<'input>(
    input: AppearanceAuthoringBatch<'input>,
    resource_snapshots: ResourceSnapshotAuthoringTable<'input>,
) -> Result<u64, AppearanceAuthoringSemanticError> {
    validate_resource_snapshot_table(resource_snapshots)?;
    for (image, authored) in input.images().iter().enumerate() {
        let (bytes, _) = resolve_image_payload(image, *authored, resource_snapshots)?;
        validate_encoded_payload_limit(image, bytes.len())?;
    }
    let image_plan =
        CapacityPlan::for_type::<ValidatedAppearanceImagePlan<'input>>(0, 0, input.images().len())
            .map_err(|cause| AppearanceAuthoringSemanticError::ScratchCapacity {
                resource: "validated appearance image plans",
                cause,
            })?;
    let material_plan =
        CapacityPlan::for_type::<ValidatedAppearanceMaterialPlan>(0, 0, input.materials().len())
            .map_err(|cause| AppearanceAuthoringSemanticError::ScratchCapacity {
                resource: "validated appearance material plans",
                cause,
            })?;
    let binding_plan =
        CapacityPlan::for_type::<ValidatedAppearanceBindingPlan>(0, 0, input.bindings().len())
            .map_err(|cause| AppearanceAuthoringSemanticError::ScratchCapacity {
                resource: "validated appearance binding plans",
                cause,
            })?;
    checked_sum_bytes(
        [
            scratch_envelope(image_plan, "validated appearance image plans")?,
            scratch_envelope(material_plan, "validated appearance material plans")?,
            scratch_envelope(binding_plan, "validated appearance binding plans")?,
        ],
        "appearance shape scratch envelope",
    )
}

pub(in crate::canonical_scene::store) fn validate_appearance_batch_with_control<'input, C>(
    input: AppearanceAuthoringBatch<'input>,
    resource_snapshots: ResourceSnapshotAuthoringTable<'input>,
    limits: SceneValidationLimits,
    control: &mut C,
) -> Result<ValidatedAppearanceBatch<'input, C::Reservation>, AppearanceAuthoringFailure<C::Error>>
where
    C: CanonicalAuthoringControl,
{
    validate_appearance_batch_inner(input, resource_snapshots, limits, control)
}

fn validate_appearance_batch_inner<'input, C>(
    input: AppearanceAuthoringBatch<'input>,
    resource_snapshots: ResourceSnapshotAuthoringTable<'input>,
    limits: SceneValidationLimits,
    control: &mut C,
) -> Result<ValidatedAppearanceBatch<'input, C::Reservation>, AppearanceAuthoringFailure<C::Error>>
where
    C: CanonicalAuthoringControl,
{
    let mut context = AppearanceValidationContext::new(limits, control);
    context.poll_now()?;
    validate_resource_snapshot_table(resource_snapshots)?;

    let image_count = collection_count(input.images().len(), "images")?;
    let sampler_count = collection_count(input.samplers().len(), "samplers")?;
    let texture_count = collection_count(input.textures().len(), "textures")?;
    let material_count = collection_count(input.materials().len(), "materials")?;
    let binding_count = collection_count(input.bindings().len(), "material bindings")?;
    let entity_count = checked_sum(
        [
            u64::from(image_count),
            u64::from(sampler_count),
            u64::from(texture_count),
            u64::from(material_count),
            u64::from(binding_count),
        ],
        "appearance entities",
    )?;
    enforce_limit::<C::Error>(
        AppearanceAuthoringLimit::Entities,
        entity_count,
        limits.max_entities(),
    )?;

    let structural_bytes = appearance_structural_bytes(input, resource_snapshots, &mut context)?;
    context.set_structural_bytes(structural_bytes)?;

    let image_plan = context.scratch_plan::<ValidatedAppearanceImagePlan<'input>>(
        input.images().len(),
        "validated appearance image plans",
    )?;
    let material_plan = context.scratch_plan::<ValidatedAppearanceMaterialPlan>(
        input.materials().len(),
        "validated appearance material plans",
    )?;
    let binding_plan = context.scratch_plan::<ValidatedAppearanceBindingPlan>(
        input.bindings().len(),
        "validated appearance binding plans",
    )?;
    let scratch_envelope = checked_sum_bytes(
        [
            scratch_envelope(image_plan, "validated appearance image plans")?,
            scratch_envelope(material_plan, "validated appearance material plans")?,
            scratch_envelope(binding_plan, "validated appearance binding plans")?,
        ],
        "appearance scratch envelope",
    )?;
    let mut reservation = context.reserve_scratch(scratch_envelope)?;
    let (image_plans, image_plan_bytes) =
        context.allocate_scratch(image_plan, "validated appearance image plans")?;
    let (material_plans, material_plan_bytes) =
        context.allocate_scratch(material_plan, "validated appearance material plans")?;
    let (binding_plans, binding_plan_bytes) =
        context.allocate_scratch(binding_plan, "validated appearance binding plans")?;
    let scratch_actual = checked_sum_bytes(
        [image_plan_bytes, material_plan_bytes, binding_plan_bytes],
        "appearance scratch allocation",
    )?;
    context.resize_scratch(&mut reservation, scratch_actual)?;
    let mut storage = AppearanceValidationScratch {
        image_plans,
        material_plans,
        binding_plans,
        reservation,
    };
    let binding_semantic_stride = collection_count(
        APPEARANCE_BINDING_SEMANTIC_STRIDE,
        "binding semantic scalar stride",
    )?;
    let material_factor_stride = collection_count(
        APPEARANCE_MATERIAL_FACTOR_STRIDE,
        "material factor scalar stride",
    )?;
    let binding_semantic_scalar_count = binding_count.checked_mul(binding_semantic_stride).ok_or(
        AppearanceAuthoringSemanticError::AccountingOverflow {
            dimension: "binding semantic scalar count",
        },
    )?;
    let material_factor_scalar_count = material_count.checked_mul(material_factor_stride).ok_or(
        AppearanceAuthoringSemanticError::AccountingOverflow {
            dimension: "material factor scalar count",
        },
    )?;

    let mut payload_offset = 0_u32;
    let mut media_type_offset = 0_u32;
    for (image_index, image) in input.images().iter().enumerate() {
        let (bytes, media_type) = context.run_unit(SceneValidationWork::Entity, || {
            let (bytes, media_type) =
                resolve_image_payload(image_index, *image, resource_snapshots)?;
            if bytes.is_empty() {
                return Err(AppearanceAuthoringSemanticError::EmptyEncodedPayload {
                    image: image_index,
                });
            }
            validate_encoded_payload_limit(image_index, bytes.len())?;
            Ok::<_, AppearanceAuthoringSemanticError>((bytes, media_type))
        })??;
        let payload_count = collection_count(bytes.len(), "encoded image payload bytes")?;
        let next_payload_offset = payload_offset.checked_add(payload_count).ok_or(
            AppearanceAuthoringSemanticError::AccountingOverflow {
                dimension: "encoded image payload byte offset",
            },
        )?;
        context.add_payload_bytes(u64::from(payload_count))?;
        for byte in bytes {
            context.run_unit(SceneValidationWork::PayloadCell, || *byte)?;
        }

        let (media_type_count, media_type_present) = match media_type {
            Presence::Absent => (0_u32, false),
            Presence::Present(media_type) => {
                let media_type_count =
                    collection_count(media_type.as_str().len(), "image media type bytes")?;
                context.add_payload_bytes(u64::from(media_type_count))?;
                for byte in media_type.as_str().as_bytes() {
                    context.run_unit(SceneValidationWork::PayloadCell, || *byte)?;
                }
                (media_type_count, true)
            }
        };
        let next_media_type_offset = media_type_offset.checked_add(media_type_count).ok_or(
            AppearanceAuthoringSemanticError::AccountingOverflow {
                dimension: "image media type byte offset",
            },
        )?;
        storage.image_plans.push(ValidatedAppearanceImagePlan {
            input_index: image_index,
            payload: bytes,
            payload_offset,
            payload_count,
            media_type_offset,
            media_type_count,
            media_type_present,
        });
        payload_offset = next_payload_offset;
        media_type_offset = next_media_type_offset;
    }

    for sampler in input.samplers() {
        context.run_unit(SceneValidationWork::Entity, || *sampler)?;
    }

    for (texture_index, texture) in input.textures().iter().copied().enumerate() {
        context.run_unit(SceneValidationWork::Entity, || {
            if texture.image().index() >= image_count {
                return Err(AppearanceAuthoringSemanticError::TextureImageOutOfRange {
                    texture: texture_index,
                    image: texture.image().index(),
                    image_count,
                });
            }
            if texture.sampler().index() >= sampler_count {
                return Err(AppearanceAuthoringSemanticError::TextureSamplerOutOfRange {
                    texture: texture_index,
                    sampler: texture.sampler().index(),
                    sampler_count,
                });
            }
            let _ = u32_to_usize(texture.image().index(), "texture image index")?;
            let _ = u32_to_usize(texture.sampler().index(), "texture sampler index")?;
            Ok::<(), AppearanceAuthoringSemanticError>(())
        })??;
        context.add_relations(2)?;
    }

    let mut source_key_offset = 0_u32;
    let mut binding_semantic_scalar_offset = 0_u32;
    for (binding_index, binding) in input.bindings().iter().copied().enumerate() {
        let semantics = binding.semantics();
        let (binding_plan, next_source_key_offset, next_binding_semantic_scalar_offset, source_key) =
            context.run_unit(SceneValidationWork::Entity, || {
                if binding.texture().index() >= texture_count {
                    return Err(AppearanceAuthoringSemanticError::BindingTextureOutOfRange {
                        binding: binding_index,
                        texture: binding.texture().index(),
                        texture_count,
                    });
                }
                let _ = u32_to_usize(binding.texture().index(), "binding texture index")?;
                if let Some(behavior) = namespaced_binding_behavior(semantics) {
                    return Err(
                        AppearanceAuthoringSemanticError::NamespacedBindingBehaviorUnsupported {
                            binding: binding_index,
                            behavior,
                        },
                    );
                }
                validate_texture_binding_semantics(semantics).map_err(|source| {
                    AppearanceAuthoringSemanticError::InvalidBindingSemantics {
                        binding: binding_index,
                        source,
                    }
                })?;
                let Some(role_tag) = encode_appearance_texture_role(binding.role()) else {
                    return Err(
                        AppearanceAuthoringSemanticError::NamespacedTextureRoleUnsupported {
                            binding: binding_index,
                        },
                    );
                };
                let Some((coordinate_source_tag, coordinate_set)) =
                    encode_appearance_coordinate_source(semantics.coordinate_source())
                else {
                    return Err(AppearanceAuthoringSemanticError::BindingEncodingInvariant {
                        binding: binding_index,
                    });
                };
                let crate::appearance::TextureChannelMapping::Swizzle(swizzle) =
                    semantics.channel_mapping()
                else {
                    return Err(AppearanceAuthoringSemanticError::BindingEncodingInvariant {
                        binding: binding_index,
                    });
                };
                let mut channel_tags = [APPEARANCE_CHANNEL_ZERO; 4];
                for (channel_index, channel) in swizzle.channels().iter().copied().enumerate() {
                    channel_tags[channel_index] = encode_appearance_channel(channel);
                }
                let Some(color_interpretation_tag) =
                    encode_appearance_color_interpretation(semantics.color_interpretation())
                else {
                    return Err(AppearanceAuthoringSemanticError::BindingEncodingInvariant {
                        binding: binding_index,
                    });
                };
                let Some(blend_operation_tag) =
                    encode_appearance_blend_operation(semantics.blend())
                else {
                    return Err(AppearanceAuthoringSemanticError::BindingEncodingInvariant {
                        binding: binding_index,
                    });
                };
                let source_key = semantics.source_key().map(|key| key.as_str());
                let source_key_count = match source_key {
                    Some(source_key) => collection_count(source_key.len(), "source key bytes")?,
                    None => 0,
                };
                let next_source_key_offset = source_key_offset
                    .checked_add(source_key_count)
                    .ok_or(AppearanceAuthoringSemanticError::AccountingOverflow {
                        dimension: "source key byte offset",
                    })?;
                let next_binding_semantic_scalar_offset = binding_semantic_scalar_offset
                    .checked_add(binding_semantic_stride)
                    .ok_or(AppearanceAuthoringSemanticError::AccountingOverflow {
                        dimension: "binding semantic scalar offset",
                    })?;
                Ok::<_, AppearanceAuthoringSemanticError>((
                    ValidatedAppearanceBindingPlan {
                        input_index: binding_index,
                        role_tag,
                        coordinate_source_tag,
                        coordinate_set,
                        channel_lane_count: swizzle.lane_count(),
                        channel_tags,
                        color_interpretation_tag,
                        semantic_scalar_offset: binding_semantic_scalar_offset,
                        blend_operation_tag,
                        source_key_offset,
                        source_key_count,
                    },
                    next_source_key_offset,
                    next_binding_semantic_scalar_offset,
                    source_key,
                ))
            })??;
        for scalar in 0..binding_semantic_stride {
            context.run_unit(SceneValidationWork::PayloadCell, || scalar)?;
        }
        if let Some(source_key) = source_key {
            for byte in source_key.as_bytes() {
                context.run_unit(SceneValidationWork::PayloadCell, || *byte)?;
            }
        }
        context.add_payload_cells_and_bytes(
            u64::from(binding_semantic_stride),
            u64::from(binding_semantic_stride) * LOGICAL_F32_BYTES,
        )?;
        context.add_payload_bytes(u64::from(binding_plan.source_key_count))?;
        storage.binding_plans.push(binding_plan);
        source_key_offset = next_source_key_offset;
        binding_semantic_scalar_offset = next_binding_semantic_scalar_offset;
        context.add_relations(1)?;
    }

    let mut expected_binding_offset = 0_u32;
    let mut material_factor_offset = 0_u32;
    let mut material_name_offset = 0_u32;
    for (material_index, material) in input.materials().iter().copied().enumerate() {
        let semantics = material.semantics();
        let (
            material_plan,
            end,
            binding_start,
            binding_end,
            range_count,
            next_material_factor_offset,
            next_material_name_offset,
            name,
        ) = context.run_unit(SceneValidationWork::Entity, || {
            if let Some(behavior) = namespaced_material_behavior(semantics) {
                return Err(
                    AppearanceAuthoringSemanticError::NamespacedMaterialBehaviorUnsupported {
                        material: material_index,
                        behavior,
                    },
                );
            }
            validate_material_semantics(semantics).map_err(|source| {
                AppearanceAuthoringSemanticError::InvalidMaterialSemantics {
                    material: material_index,
                    source,
                }
            })?;
            let Some(shading_model_tag) =
                encode_appearance_shading_model(semantics.shading_model())
            else {
                return Err(
                    AppearanceAuthoringSemanticError::MaterialEncodingInvariant {
                        material: material_index,
                    },
                );
            };
            let Some(alpha_mode_tag) = encode_appearance_alpha_mode(semantics.alpha_mode()) else {
                return Err(
                    AppearanceAuthoringSemanticError::MaterialEncodingInvariant {
                        material: material_index,
                    },
                );
            };
            let alpha_cutoff = semantics.alpha_mode().mask_cutoff().unwrap_or(0.0);
            let factors = semantics.factors();
            let pbr_metallic_roughness_presence = if factors.pbr_metallic_roughness().is_some() {
                APPEARANCE_PRESENCE_PRESENT
            } else {
                APPEARANCE_PRESENCE_ABSENT
            };
            let pbr_specular_glossiness_presence = if factors.pbr_specular_glossiness().is_some() {
                APPEARANCE_PRESENCE_PRESENT
            } else {
                APPEARANCE_PRESENCE_ABSENT
            };
            let legacy_presence = if factors.legacy().is_some() {
                APPEARANCE_PRESENCE_PRESENT
            } else {
                APPEARANCE_PRESENCE_ABSENT
            };
            let range = material.bindings();
            if range.offset() != expected_binding_offset {
                return Err(
                    AppearanceAuthoringSemanticError::MaterialBindingRangeOrder {
                        material: material_index,
                        expected: expected_binding_offset,
                        actual: range.offset(),
                    },
                );
            }
            let Some(end) = range.offset().checked_add(range.count()) else {
                return Err(
                    AppearanceAuthoringSemanticError::MaterialBindingRangeOutOfBounds {
                        material: material_index,
                        binding_count: input.bindings().len(),
                    },
                );
            };
            if end > binding_count {
                return Err(
                    AppearanceAuthoringSemanticError::MaterialBindingRangeOutOfBounds {
                        material: material_index,
                        binding_count: input.bindings().len(),
                    },
                );
            }
            let binding_start = u32_to_usize(range.offset(), "material binding offset")?;
            let binding_end = u32_to_usize(end, "material binding end")?;
            let next_material_factor_offset = material_factor_offset
                .checked_add(material_factor_stride)
                .ok_or(AppearanceAuthoringSemanticError::AccountingOverflow {
                    dimension: "material factor scalar offset",
                })?;
            let name = match material.name() {
                Presence::Absent => None,
                Presence::Present(name) => {
                    validate_entity_name_str(name).map_err(|source| {
                        AppearanceAuthoringSemanticError::InvalidMaterialName {
                            material: material_index,
                            source,
                        }
                    })?;
                    Some(name)
                }
            };
            let name_count = match name {
                Some(name) => collection_count(name.len(), "material name bytes")?,
                None => 0,
            };
            let next_material_name_offset = material_name_offset.checked_add(name_count).ok_or(
                AppearanceAuthoringSemanticError::AccountingOverflow {
                    dimension: "material name byte offset",
                },
            )?;
            Ok::<_, AppearanceAuthoringSemanticError>((
                ValidatedAppearanceMaterialPlan {
                    input_index: material_index,
                    binding_start,
                    binding_end,
                    shading_model_tag,
                    alpha_mode_tag,
                    alpha_cutoff,
                    double_sided_tag: if semantics.double_sided() {
                        APPEARANCE_PRESENCE_PRESENT
                    } else {
                        APPEARANCE_PRESENCE_ABSENT
                    },
                    pbr_metallic_roughness_presence,
                    pbr_specular_glossiness_presence,
                    legacy_presence,
                    factor_offset: material_factor_offset,
                    name_offset: material_name_offset,
                    name_count,
                },
                end,
                binding_start,
                binding_end,
                range.count(),
                next_material_factor_offset,
                next_material_name_offset,
                name,
            ))
        })??;
        let material_bindings = &input.bindings()[binding_start..binding_end];
        let mut binding_validator =
            MaterialTextureBindingSemanticsValidator::new(material_bindings.len());
        for binding in material_bindings {
            context.run_unit(SceneValidationWork::Traversal, || {
                binding_validator
                    .validate_next(binding.semantics())
                    .map_err(|source| {
                        AppearanceAuthoringSemanticError::InvalidMaterialBindingSemantics {
                            material: material_index,
                            source,
                        }
                    })
            })??;
        }
        for scalar in 0..material_factor_stride {
            context.run_unit(SceneValidationWork::PayloadCell, || scalar)?;
        }
        if let Some(name) = name {
            for byte in name.as_bytes() {
                context.run_unit(SceneValidationWork::PayloadCell, || *byte)?;
            }
        }
        context.add_payload_cells_and_bytes(
            u64::from(material_factor_stride),
            u64::from(material_factor_stride) * LOGICAL_F32_BYTES,
        )?;
        context.add_payload_bytes(u64::from(material_plan.name_count))?;
        storage.material_plans.push(material_plan);
        material_factor_offset = next_material_factor_offset;
        material_name_offset = next_material_name_offset;
        context.add_relations(u64::from(range_count))?;
        expected_binding_offset = end;
    }
    context.run_unit(SceneValidationWork::Traversal, || {
        if expected_binding_offset == binding_count {
            return Ok(());
        }
        let unowned = binding_count.checked_sub(expected_binding_offset).ok_or(
            AppearanceAuthoringSemanticError::AccountingOverflow {
                dimension: "unowned material binding count",
            },
        )?;
        Err(AppearanceAuthoringSemanticError::UnownedBindings {
            unowned: u32_to_usize(unowned, "unowned material bindings")?,
        })
    })??;
    if binding_semantic_scalar_offset != binding_semantic_scalar_count
        || material_factor_offset != material_factor_scalar_count
    {
        return Err(AppearanceAuthoringSemanticError::AccountingOverflow {
            dimension: "appearance semantic payload closure",
        }
        .into());
    }

    context.finish()?;
    let payload_block_count = block_for_count(payload_offset)
        .checked_add(block_for_count(media_type_offset))
        .and_then(|count| count.checked_add(block_for_count(source_key_offset)))
        .and_then(|count| count.checked_add(block_for_count(material_name_offset)))
        .and_then(|count| count.checked_add(block_for_count(binding_semantic_scalar_count)))
        .and_then(|count| count.checked_add(block_for_count(material_factor_scalar_count)))
        .ok_or(AppearanceAuthoringSemanticError::AccountingOverflow {
            dimension: "appearance payload block count",
        })?;
    let AppearanceValidationScratch {
        image_plans,
        material_plans,
        binding_plans,
        reservation,
    } = storage;
    let compiled = AppearanceValidationStorage {
        image_count,
        sampler_count,
        texture_count,
        material_count,
        binding_count,
        image_payload_byte_count: payload_offset,
        media_type_byte_count: media_type_offset,
        source_key_byte_count: source_key_offset,
        material_name_byte_count: material_name_offset,
        binding_semantic_scalar_count,
        material_factor_scalar_count,
        payload_block_count,
        entity_count,
        edge_count: context.edges,
        reference_count: context.references,
        semantic_cell_count: context.semantic_cells,
        canonical_bytes: context.canonical_bytes,
        validation_work: context.work,
        scratch_bytes: scratch_actual,
        image_plans,
        material_plans,
        binding_plans,
        #[cfg(test)]
        storage_drop_probe: None,
        reservation,
    };
    Ok(ValidatedAppearanceBatch::new(input, compiled))
}

fn namespaced_binding_behavior(
    semantics: TextureBindingSemanticsRef<'_>,
) -> Option<AppearanceBehaviorKind> {
    if semantics.role().schema_id().is_some() {
        Some(AppearanceBehaviorKind::TextureRole)
    } else if semantics.coordinate_source().schema_id().is_some() {
        Some(AppearanceBehaviorKind::CoordinateSource)
    } else if semantics.channel_mapping().schema_id().is_some() {
        Some(AppearanceBehaviorKind::ChannelMapping)
    } else if semantics.color_interpretation().schema_id().is_some() {
        Some(AppearanceBehaviorKind::ColorInterpretation)
    } else if semantics.blend().schema_id().is_some() {
        Some(AppearanceBehaviorKind::BlendOperation)
    } else {
        None
    }
}

fn namespaced_material_behavior(
    semantics: MaterialSemanticsRef<'_>,
) -> Option<AppearanceBehaviorKind> {
    if semantics.shading_model().schema_id().is_some() {
        Some(AppearanceBehaviorKind::ShadingModel)
    } else if semantics.alpha_mode().schema_id().is_some() {
        Some(AppearanceBehaviorKind::AlphaMode)
    } else {
        None
    }
}

pub(in crate::canonical_scene::store) fn validate_encoded_payload_limit(
    image: usize,
    actual: usize,
) -> Result<(), AppearanceAuthoringSemanticError> {
    if actual <= MAX_ENCODED_IMAGE_BYTES {
        Ok(())
    } else {
        Err(AppearanceAuthoringSemanticError::EncodedPayloadTooLarge {
            image,
            actual,
            maximum: MAX_ENCODED_IMAGE_BYTES,
        })
    }
}

fn validate_resource_snapshot_table(
    resource_snapshots: ResourceSnapshotAuthoringTable<'_>,
) -> Result<(), AppearanceAuthoringSemanticError> {
    let snapshot_count = resource_snapshots.snapshots().len();
    let resource_count = resource_snapshots.resource_count();
    if snapshot_count == 0 || snapshot_count == resource_count {
        Ok(())
    } else {
        Err(
            AppearanceAuthoringSemanticError::ResourceSnapshotCardinalityMismatch {
                snapshot_count,
                resource_count,
            },
        )
    }
}

fn resolve_image_payload<'input>(
    image: usize,
    authored: ImageAuthoring<'input>,
    resource_snapshots: ResourceSnapshotAuthoringTable<'input>,
) -> Result<
    (
        &'input [u8],
        Presence<&'input crate::extension::ExtensionMediaType>,
    ),
    AppearanceAuthoringSemanticError,
> {
    match authored.payload() {
        ImagePayloadAuthoring::Encoded { bytes, media_type } => Ok((bytes, media_type)),
        ImagePayloadAuthoring::SessionResource {
            resource,
            media_type,
        } => {
            let resource_index = u32_to_usize(resource.index(), "image session resource index")?;
            if resource_index >= resource_snapshots.resource_count() {
                return Err(AppearanceAuthoringSemanticError::ImageResourceOutOfRange {
                    image,
                    resource: resource.index(),
                    resource_count: resource_snapshots.resource_count(),
                });
            }
            let Some(snapshot) = resource_snapshots.snapshots().get(resource_index) else {
                return Err(
                    AppearanceAuthoringSemanticError::ImageResourceSnapshotAbsent {
                        image,
                        resource: resource.index(),
                    },
                );
            };
            let Presence::Present(bytes) = snapshot.bytes() else {
                return Err(
                    AppearanceAuthoringSemanticError::ImageResourceSnapshotAbsent {
                        image,
                        resource: resource.index(),
                    },
                );
            };
            Ok((bytes, media_type))
        }
    }
}

fn appearance_structural_bytes<'input, C>(
    input: AppearanceAuthoringBatch<'input>,
    resource_snapshots: ResourceSnapshotAuthoringTable<'input>,
    context: &mut AppearanceValidationContext<'_, C>,
) -> AppearanceValidationResult<u64, C>
where
    C: CanonicalAuthoringControl,
{
    let image_count = collection_count(input.images().len(), "images")?;
    let sampler_count = collection_count(input.samplers().len(), "samplers")?;
    let texture_count = collection_count(input.textures().len(), "textures")?;
    let material_count = collection_count(input.materials().len(), "materials")?;
    let binding_count = collection_count(input.bindings().len(), "material bindings")?;

    let mut payload_bytes = 0_u64;
    let mut media_type_bytes = 0_u64;
    let mut image_rows = 0_u64;
    for (image_index, image) in input.images().iter().copied().enumerate() {
        context.run_unit(SceneValidationWork::Traversal, || {
            let (bytes, media_type) =
                resolve_image_payload(image_index, image, resource_snapshots)?;
            let payload_count = usize_to_u64(bytes.len(), "encoded image payload bytes")?;
            let media_count = match media_type {
                Presence::Absent => 0,
                Presence::Present(media_type) => {
                    usize_to_u64(media_type.as_str().len(), "image media type bytes")?
                }
            };
            payload_bytes = payload_bytes.checked_add(payload_count).ok_or(
                AppearanceAuthoringSemanticError::AccountingOverflow {
                    dimension: "encoded image payload bytes",
                },
            )?;
            media_type_bytes = media_type_bytes.checked_add(media_count).ok_or(
                AppearanceAuthoringSemanticError::AccountingOverflow {
                    dimension: "image media type bytes",
                },
            )?;
            image_rows = image_rows
                .checked_add(IMAGE_ROW_FIXED_BYTES)
                .and_then(|bytes| bytes.checked_add(reference_slot_bytes(payload_count != 0)))
                .and_then(|bytes| bytes.checked_add(reference_slot_bytes(media_count != 0)))
                .ok_or(AppearanceAuthoringSemanticError::AccountingOverflow {
                    dimension: "canonical image records",
                })?;
            Ok::<(), AppearanceAuthoringSemanticError>(())
        })??;
    }

    let mut material_name_bytes = 0_u64;
    let mut material_rows = 0_u64;
    for material in input.materials() {
        context.run_unit(SceneValidationWork::Traversal, || {
            let name_count = match material.name() {
                Presence::Absent => 0,
                Presence::Present(name) => usize_to_u64(name.len(), "material name bytes")?,
            };
            material_name_bytes = material_name_bytes.checked_add(name_count).ok_or(
                AppearanceAuthoringSemanticError::AccountingOverflow {
                    dimension: "material name bytes",
                },
            )?;
            material_rows = material_rows
                .checked_add(MATERIAL_ROW_FIXED_BYTES)
                .and_then(|bytes| {
                    bytes.checked_add(reference_slot_bytes(material.bindings().count() != 0))
                })
                .and_then(|bytes| bytes.checked_add(LOGICAL_REFERENCE_BYTES))
                .and_then(|bytes| bytes.checked_add(reference_slot_bytes(name_count != 0)))
                .ok_or(AppearanceAuthoringSemanticError::AccountingOverflow {
                    dimension: "canonical material records",
                })?;
            Ok::<(), AppearanceAuthoringSemanticError>(())
        })??;
    }

    let mut source_key_bytes = 0_u64;
    let mut binding_rows = 0_u64;
    for binding in input.bindings() {
        context.run_unit(SceneValidationWork::Traversal, || {
            let source_key_count = binding
                .semantics()
                .source_key()
                .map_or(0, |key| key.as_str().len());
            let source_key_count = usize_to_u64(source_key_count, "source key bytes")?;
            source_key_bytes = source_key_bytes.checked_add(source_key_count).ok_or(
                AppearanceAuthoringSemanticError::AccountingOverflow {
                    dimension: "source key bytes",
                },
            )?;
            binding_rows = binding_rows
                .checked_add(BINDING_ROW_FIXED_BYTES)
                .and_then(|bytes| bytes.checked_add(LOGICAL_REFERENCE_BYTES))
                .and_then(|bytes| bytes.checked_add(reference_slot_bytes(source_key_count != 0)))
                .ok_or(AppearanceAuthoringSemanticError::AccountingOverflow {
                    dimension: "canonical material binding records",
                })?;
            Ok::<(), AppearanceAuthoringSemanticError>(())
        })??;
    }

    let directory_reference_bytes = checked_sum_bytes(
        [
            reference_slot_bytes(image_count != 0),
            reference_slot_bytes(sampler_count != 0),
            reference_slot_bytes(texture_count != 0),
            reference_slot_bytes(material_count != 0),
            reference_slot_bytes(binding_count != 0),
            reference_slot_bytes(payload_bytes != 0),
            reference_slot_bytes(media_type_bytes != 0),
            reference_slot_bytes(source_key_bytes != 0),
            reference_slot_bytes(material_name_bytes != 0),
            reference_slot_bytes(binding_count != 0),
            reference_slot_bytes(material_count != 0),
        ],
        "canonical appearance directory references",
    )?;
    Ok(checked_sum_bytes(
        [
            APPEARANCE_DIRECTORY_FIXED_BYTES,
            directory_reference_bytes,
            image_rows,
            checked_bytes(
                u64::from(sampler_count),
                SAMPLER_ROW_BYTES,
                "canonical sampler records",
            )?,
            checked_bytes(
                u64::from(texture_count),
                TEXTURE_ROW_BYTES,
                "canonical texture records",
            )?,
            material_rows,
            binding_rows,
        ],
        "canonical appearance structural footprint",
    )?)
}

const fn reference_slot_bytes(has_reference: bool) -> u64 {
    if has_reference {
        LOGICAL_REFERENCE_BYTES
    } else {
        LOGICAL_U8_BYTES
    }
}

fn checked_bytes(
    count: u64,
    width: u64,
    dimension: &'static str,
) -> Result<u64, AppearanceAuthoringSemanticError> {
    count
        .checked_mul(width)
        .ok_or(AppearanceAuthoringSemanticError::AccountingOverflow { dimension })
}

fn scratch_envelope(
    plan: CapacityPlan,
    resource: &'static str,
) -> Result<u64, AppearanceAuthoringSemanticError> {
    plan.envelope_delta_bytes()
        .map_err(|cause| AppearanceAuthoringSemanticError::ScratchCapacity { resource, cause })
}

fn checked_sum<const N: usize>(
    values: [u64; N],
    dimension: &'static str,
) -> Result<u64, AppearanceAuthoringSemanticError> {
    values.into_iter().try_fold(0_u64, |total, value| {
        total
            .checked_add(value)
            .ok_or(AppearanceAuthoringSemanticError::AccountingOverflow { dimension })
    })
}

fn checked_sum_bytes<const N: usize>(
    values: [u64; N],
    dimension: &'static str,
) -> Result<u64, AppearanceAuthoringSemanticError> {
    checked_sum(values, dimension)
}

fn collection_count(
    count: usize,
    collection: &'static str,
) -> Result<u32, AppearanceAuthoringSemanticError> {
    u32::try_from(count)
        .map_err(|_| AppearanceAuthoringSemanticError::CountDomainTooLarge { collection })
}

fn usize_to_u64(
    value: usize,
    dimension: &'static str,
) -> Result<u64, AppearanceAuthoringSemanticError> {
    u64::try_from(value)
        .map_err(|_| AppearanceAuthoringSemanticError::AccountingOverflow { dimension })
}

fn u32_to_usize(
    value: u32,
    dimension: &'static str,
) -> Result<usize, AppearanceAuthoringSemanticError> {
    usize::try_from(value)
        .map_err(|_| AppearanceAuthoringSemanticError::AccountingOverflow { dimension })
}

fn block_for_count(count: u32) -> u32 {
    u32::from(count != 0)
}

fn enforce_limit<E>(
    limit: AppearanceAuthoringLimit,
    actual: u64,
    maximum: u64,
) -> Result<(), AppearanceAuthoringFailure<E>>
where
    E: Error + Send + Sync + 'static,
{
    if actual <= maximum {
        Ok(())
    } else {
        Err(AppearanceAuthoringSemanticError::LimitExceeded {
            limit,
            actual,
            maximum,
        }
        .into())
    }
}
