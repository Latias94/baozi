//! Small borrowed iterators project inline domain values into the unchanged snapshot order.
//! Absent optional factor groups emit canonical positive zero; authored component bits remain exact.

use std::fmt;
use std::iter::FusedIterator;
use std::ops::Range;

use super::*;

#[derive(Clone)]
#[must_use = "the factor iterator must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneAppearanceMaterialFactors<'scene> {
    factors: &'scene MaterialFactors,
    remaining: Range<usize>,
}

impl<'scene> NodeSceneAppearanceMaterialFactors<'scene> {
    pub(super) fn new(factors: &'scene MaterialFactors) -> Self {
        Self {
            factors,
            remaining: 0..MATERIAL_FACTOR_SCALARS,
        }
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        exhausted(self.remaining)
    }
}

impl Iterator for NodeSceneAppearanceMaterialFactors<'_> {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        self.remaining
            .next()
            .map(|index| factor_scalar(self.factors, index))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.remaining.size_hint()
    }
}

impl DoubleEndedIterator for NodeSceneAppearanceMaterialFactors<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.remaining
            .next_back()
            .map(|index| factor_scalar(self.factors, index))
    }
}

impl ExactSizeIterator for NodeSceneAppearanceMaterialFactors<'_> {}
impl FusedIterator for NodeSceneAppearanceMaterialFactors<'_> {}

impl fmt::Debug for NodeSceneAppearanceMaterialFactors<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.clone()).finish()
    }
}

fn factor_scalar(factors: &MaterialFactors, index: usize) -> f32 {
    match index {
        0 => factors.emissive().x,
        1 => factors.emissive().y,
        2 => factors.emissive().z,
        3..=8 => factors
            .pbr_metallic_roughness()
            .map_or(0.0, |value| match index {
                3 => value.base_color().r,
                4 => value.base_color().g,
                5 => value.base_color().b,
                6 => value.base_color().a,
                7 => value.metallic(),
                _ => value.roughness(),
            }),
        9..=16 => factors
            .pbr_specular_glossiness()
            .map_or(0.0, |value| match index {
                9 => value.diffuse().r,
                10 => value.diffuse().g,
                11 => value.diffuse().b,
                12 => value.diffuse().a,
                13 => value.specular().x,
                14 => value.specular().y,
                15 => value.specular().z,
                _ => value.glossiness(),
            }),
        17..=30 => factors.legacy().map_or(0.0, |value| match index {
            17 => value.ambient().x,
            18 => value.ambient().y,
            19 => value.ambient().z,
            20 => value.diffuse().x,
            21 => value.diffuse().y,
            22 => value.diffuse().z,
            23 => value.specular().x,
            24 => value.specular().y,
            25 => value.specular().z,
            26 => value.emissive().x,
            27 => value.emissive().y,
            28 => value.emissive().z,
            29 => value.shininess(),
            _ => value.opacity(),
        }),
        31 => factors.supplemental_pbr().clearcoat(),
        32 => factors.supplemental_pbr().clearcoat_roughness(),
        33 => factors.supplemental_pbr().transmission(),
        34 => factors.supplemental_pbr().thickness(),
        35 => factors.supplemental_pbr().sheen_color().x,
        36 => factors.supplemental_pbr().sheen_color().y,
        37 => factors.supplemental_pbr().sheen_color().z,
        38 => factors.supplemental_pbr().sheen_roughness(),
        _ => unreachable!("the private factor iterator selects only the declared transcript width"),
    }
}

#[derive(Clone)]
#[must_use = "the binding scalar iterator must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneAppearanceBindingScalars<'scene> {
    binding: &'scene StoredBinding,
    remaining: Range<usize>,
}

impl<'scene> NodeSceneAppearanceBindingScalars<'scene> {
    pub(super) fn new(binding: &'scene StoredBinding) -> Self {
        Self {
            binding,
            remaining: 0..BINDING_SEMANTIC_SCALARS,
        }
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        exhausted(self.remaining)
    }
}

impl Iterator for NodeSceneAppearanceBindingScalars<'_> {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        self.remaining
            .next()
            .map(|index| binding_scalar(self.binding, index))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.remaining.size_hint()
    }
}

impl DoubleEndedIterator for NodeSceneAppearanceBindingScalars<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.remaining
            .next_back()
            .map(|index| binding_scalar(self.binding, index))
    }
}

impl ExactSizeIterator for NodeSceneAppearanceBindingScalars<'_> {}
impl FusedIterator for NodeSceneAppearanceBindingScalars<'_> {}

impl fmt::Debug for NodeSceneAppearanceBindingScalars<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.clone()).finish()
    }
}

fn binding_scalar(binding: &StoredBinding, index: usize) -> f32 {
    match index {
        0 => binding.transform.offset().x,
        1 => binding.transform.offset().y,
        2 => binding.transform.rotation_radians(),
        3 => binding.transform.scale().x,
        4 => binding.transform.scale().y,
        5 => binding.strength.get(),
        _ => {
            unreachable!("the private binding iterator selects only the declared transcript width")
        }
    }
}

fn exhausted(remaining: Range<usize>) -> NodeSceneSemanticResult<()> {
    if !remaining.is_empty() {
        return Err(invariant(
            "appearance scalar iterator ended before its declared transcript width",
        ));
    }
    Ok(())
}
