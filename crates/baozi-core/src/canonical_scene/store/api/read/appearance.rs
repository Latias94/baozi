//! Borrowed material, binding, image, sampler, and texture views.

use std::fmt;

use crate::appearance::{
    AlphaMode, ColorInterpretation, MaterialFactors, ShadingModel, TextureBlendOperation,
    TextureChannelSwizzle, TextureCoordinateSource, TextureRole, TextureStrength, TextureTransform,
};
use crate::canonical_scene::store::construction::node_scene::semantic::appearance::{
    NodeSceneAppearanceBinding, NodeSceneAppearanceImage, NodeSceneAppearanceMaterial,
    NodeSceneAppearanceSampler, NodeSceneAppearanceTables, NodeSceneAppearanceTexture,
};
use crate::identity::{
    ImageId, MaterialId, SamplerId, SceneEntityKind, SceneLookupError, TextureId,
};
use crate::image::SamplerState;

use super::super::scene::ValidatedScene;
use super::{checked_index, expect_validated};

impl ValidatedScene {
    /// Returns the number of retained materials in constant time.
    pub fn material_count(&self) -> usize {
        self.as_node_scene().read_proof().material_count()
    }

    /// Returns the number of retained material-owned texture bindings in constant time.
    pub fn material_binding_count(&self) -> usize {
        self.as_node_scene().read_proof().material_binding_count()
    }

    /// Returns the number of retained encoded images in constant time.
    pub fn image_count(&self) -> usize {
        self.as_node_scene().read_proof().image_count()
    }

    /// Returns the number of retained samplers in constant time.
    pub fn sampler_count(&self) -> usize {
        self.as_node_scene().read_proof().sampler_count()
    }

    /// Returns the number of retained textures in constant time.
    pub fn texture_count(&self) -> usize {
        self.as_node_scene().read_proof().texture_count()
    }

    /// Iterates materials independently of every other Scene read.
    pub fn materials(
        &self,
    ) -> impl ExactSizeIterator<Item = MaterialRef<'_>> + DoubleEndedIterator + '_ {
        (0..self.material_count()).map(|index| MaterialRef::at(self, index))
    }

    /// Iterates encoded images without decoding or copying their payloads.
    pub fn images(&self) -> impl ExactSizeIterator<Item = ImageRef<'_>> + DoubleEndedIterator + '_ {
        (0..self.image_count()).map(|index| ImageRef::at(self, index))
    }

    /// Iterates samplers independently of material and texture traversal.
    pub fn samplers(
        &self,
    ) -> impl ExactSizeIterator<Item = SamplerRef<'_>> + DoubleEndedIterator + '_ {
        (0..self.sampler_count()).map(|index| SamplerRef::at(self, index))
    }

    /// Iterates textures independently of material and image traversal.
    pub fn textures(
        &self,
    ) -> impl ExactSizeIterator<Item = TextureRef<'_>> + DoubleEndedIterator + '_ {
        (0..self.texture_count()).map(|index| TextureRef::at(self, index))
    }

    /// Looks up a material after checking its Scene generation and collection range.
    pub fn material(&self, id: MaterialId) -> Result<MaterialRef<'_>, SceneLookupError> {
        let index = checked_index(
            self,
            SceneEntityKind::Material,
            id.generation(),
            id.index(),
            self.material_count(),
        )?;
        Ok(MaterialRef::at(self, index))
    }

    /// Looks up an image after checking its Scene generation and collection range.
    pub fn image(&self, id: ImageId) -> Result<ImageRef<'_>, SceneLookupError> {
        let index = checked_index(
            self,
            SceneEntityKind::Image,
            id.generation(),
            id.index(),
            self.image_count(),
        )?;
        Ok(ImageRef::at(self, index))
    }

    /// Looks up a sampler after checking its Scene generation and collection range.
    pub fn sampler(&self, id: SamplerId) -> Result<SamplerRef<'_>, SceneLookupError> {
        let index = checked_index(
            self,
            SceneEntityKind::Sampler,
            id.generation(),
            id.index(),
            self.sampler_count(),
        )?;
        Ok(SamplerRef::at(self, index))
    }

    /// Looks up a texture after checking its Scene generation and collection range.
    pub fn texture(&self, id: TextureId) -> Result<TextureRef<'_>, SceneLookupError> {
        let index = checked_index(
            self,
            SceneEntityKind::Texture,
            id.generation(),
            id.index(),
            self.texture_count(),
        )?;
        Ok(TextureRef::at(self, index))
    }
}

/// A material borrowed from an immutable validated Scene.
///
/// Every getter reads only this material's row. Names and bindings borrow the Scene for `'s`,
/// so they remain usable after the temporary material view or iterator is dropped.
#[derive(Clone, Copy)]
pub struct MaterialRef<'s> {
    scene: &'s ValidatedScene,
    index: usize,
}

impl<'s> MaterialRef<'s> {
    pub(super) const fn at(scene: &'s ValidatedScene, index: usize) -> Self {
        Self { scene, index }
    }

    /// Returns this material's generation-qualified identity.
    pub fn id(self) -> MaterialId {
        MaterialId::new(
            self.scene.generation(),
            u32::try_from(self.index).expect("validated material index fits u32"),
        )
    }

    /// Returns the authored name, preserving an absent name and an authored empty name.
    pub fn name(self) -> Option<&'s str> {
        self.read().name()
    }

    /// Returns the source shading model without reinterpreting its factor groups.
    pub fn shading_model(self) -> ShadingModel {
        *self.read().shading_model()
    }

    /// Returns the authored alpha behavior and any mask cutoff.
    pub fn alpha_mode(self) -> AlphaMode {
        *self.read().alpha_mode()
    }

    /// Returns whether both sides of the material are rendered.
    pub fn double_sided(self) -> bool {
        self.read().double_sided()
    }

    /// Returns the retained semantic factor groups, including independent legacy factors.
    pub fn factors(self) -> MaterialFactors {
        self.read().factors()
    }

    /// Returns the number of texture bindings owned by this material.
    pub fn binding_count(self) -> usize {
        self.read().binding_count()
    }

    /// Iterates this material's bindings in their retained order without scanning other rows.
    pub fn bindings(
        self,
    ) -> impl ExactSizeIterator<Item = MaterialBindingRef<'s>> + DoubleEndedIterator + 's {
        let material = self.read();
        let start = material.binding_offset();
        let end = start + material.binding_count();
        (start..end).map(move |index| MaterialBindingRef::at(self.scene, index))
    }

    fn read(self) -> NodeSceneAppearanceMaterial<'s> {
        expect_validated(tables(self.scene).material_at(self.index))
    }
}

impl fmt::Debug for MaterialRef<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MaterialRef")
            .field("id", &self.id())
            .finish_non_exhaustive()
    }
}

/// One material-owned texture use borrowed from a validated Scene.
///
/// Binding semantics describe how a material uses a texture; the texture and its encoded image
/// retain their own identities. All getters perform constant work and allocate no storage.
#[derive(Clone, Copy)]
pub struct MaterialBindingRef<'s> {
    scene: &'s ValidatedScene,
    index: usize,
}

impl<'s> MaterialBindingRef<'s> {
    pub(super) const fn at(scene: &'s ValidatedScene, index: usize) -> Self {
        Self { scene, index }
    }

    /// Returns the validated texture link directly.
    pub fn texture(self) -> TextureRef<'s> {
        TextureRef::at(self.scene, self.read().texture())
    }

    /// Returns the texture's semantic role, preserving legacy diffuse versus PBR base color.
    pub fn role(self) -> TextureRole {
        self.read().role()
    }

    /// Returns the source of texture coordinates and any UV set selection.
    pub fn coordinate_source(self) -> TextureCoordinateSource {
        *self.read().coordinate_source()
    }

    /// Returns the retained source-channel mapping.
    pub fn channel_mapping(self) -> TextureChannelSwizzle {
        self.read().channel_mapping()
    }

    /// Returns whether sampled channels represent color or data.
    pub fn color_interpretation(self) -> ColorInterpretation {
        *self.read().color_interpretation()
    }

    /// Returns the authored coordinate transform.
    pub fn transform(self) -> TextureTransform {
        self.read().transform()
    }

    /// Returns the role-specific texture strength.
    pub fn strength(self) -> TextureStrength {
        self.read().strength()
    }

    /// Returns the authored texture blend operation.
    pub fn blend(self) -> TextureBlendOperation {
        *self.read().blend()
    }

    /// Returns the optional format-owned source property key without copying its text.
    pub fn source_key(self) -> Option<&'s str> {
        self.read().source_key()
    }

    fn read(self) -> NodeSceneAppearanceBinding<'s> {
        expect_validated(tables(self.scene).binding_at(self.index))
    }
}

impl fmt::Debug for MaterialBindingRef<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MaterialBindingRef")
            .finish_non_exhaustive()
    }
}

/// An encoded image borrowed from a validated Scene; pixel decoding is left to the consumer.
#[derive(Clone, Copy)]
pub struct ImageRef<'s> {
    scene: &'s ValidatedScene,
    index: usize,
}

impl<'s> ImageRef<'s> {
    pub(super) const fn at(scene: &'s ValidatedScene, index: usize) -> Self {
        Self { scene, index }
    }

    /// Returns this image's generation-qualified identity.
    pub fn id(self) -> ImageId {
        ImageId::new(
            self.scene.generation(),
            u32::try_from(self.index).expect("validated image index fits u32"),
        )
    }

    /// Borrows the complete encoded payload without parsing or allocating.
    pub fn encoded_bytes(self) -> &'s [u8] {
        self.read().encoded_payload()
    }

    /// Returns the declared media type, if one was retained.
    pub fn media_type(self) -> Option<&'s str> {
        self.read().media_type()
    }

    fn read(self) -> NodeSceneAppearanceImage<'s> {
        expect_validated(tables(self.scene).image_at(self.index))
    }
}

impl fmt::Debug for ImageRef<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImageRef")
            .field("id", &self.id())
            .finish_non_exhaustive()
    }
}

/// Independent texture sampling behavior borrowed from a validated Scene.
#[derive(Clone, Copy)]
pub struct SamplerRef<'s> {
    scene: &'s ValidatedScene,
    index: usize,
}

impl<'s> SamplerRef<'s> {
    pub(super) const fn at(scene: &'s ValidatedScene, index: usize) -> Self {
        Self { scene, index }
    }

    /// Returns this sampler's generation-qualified identity.
    pub fn id(self) -> SamplerId {
        SamplerId::new(
            self.scene.generation(),
            u32::try_from(self.index).expect("validated sampler index fits u32"),
        )
    }

    /// Returns the retained independent minification, magnification, mipmap, and address state.
    pub fn state(self) -> SamplerState {
        self.read().state()
    }

    fn read(self) -> NodeSceneAppearanceSampler {
        expect_validated(tables(self.scene).sampler_at(self.index))
    }
}

impl fmt::Debug for SamplerRef<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SamplerRef")
            .field("id", &self.id())
            .finish_non_exhaustive()
    }
}

/// A validated image/sampler pair borrowed from its Scene.
#[derive(Clone, Copy)]
pub struct TextureRef<'s> {
    scene: &'s ValidatedScene,
    index: usize,
}

impl<'s> TextureRef<'s> {
    pub(super) const fn at(scene: &'s ValidatedScene, index: usize) -> Self {
        Self { scene, index }
    }

    /// Returns this texture's generation-qualified identity.
    pub fn id(self) -> TextureId {
        TextureId::new(
            self.scene.generation(),
            u32::try_from(self.index).expect("validated texture index fits u32"),
        )
    }

    /// Follows this texture's validated image link without another checked ID lookup.
    pub fn image(self) -> ImageRef<'s> {
        ImageRef::at(self.scene, self.read().image())
    }

    /// Follows this texture's validated sampler link without another checked ID lookup.
    pub fn sampler(self) -> SamplerRef<'s> {
        SamplerRef::at(self.scene, self.read().sampler())
    }

    fn read(self) -> NodeSceneAppearanceTexture {
        expect_validated(tables(self.scene).texture_at(self.index))
    }
}

impl fmt::Debug for TextureRef<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TextureRef")
            .field("id", &self.id())
            .finish_non_exhaustive()
    }
}

fn tables(scene: &ValidatedScene) -> NodeSceneAppearanceTables<'_> {
    let owner = scene.as_node_scene();
    expect_validated(NodeSceneAppearanceTables::try_new(
        owner.read_appearance(),
        owner.read_proof(),
    ))
}
