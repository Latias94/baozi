//! Test fixture headers over the sole production selected reconstruction engine.
use super::super::StoredAttributeSemantic;
use super::super::validation::TypedStorageValidationControl;
use super::test_selection::TestSelection;
use super::{
    EditResult, GeometryStorage, StoredAttribute, StoredAttributePayload, StoredLogicalMesh,
    StoredMeshInstance, StoredPrimitive, TableRange, TypedStorageBuildControl,
    TypedStorageBuildError, VectorValues, same_range,
};
use crate::attribute::AttributeDomain;
use crate::geometry::TopologyKind;
use baozi_operation::CapacityPlan;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NormalWritePolicy {
    Add,
    Replace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum TangentPolicy {
    Reject,
    Remove,
}

/// A closed, allocation-free index relation that can outlive the borrowed geometry plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NormalAttributeMapping {
    source_attributes: u32,
    output_attributes: u32,
    /// Source coordinate of the first target attribute ranked after Normal, or the range end.
    insertion_boundary: u32,
    normal_attribute: u32,
    source_normal: Option<u32>,
    source_tangent: Option<u32>,
    source_position: u32,
    source_smoothing_group: Option<u32>,
}

impl NormalAttributeMapping {
    #[cfg(test)]
    pub(in crate::canonical_scene::store) const fn source_attribute_count(self) -> u32 {
        self.source_attributes
    }

    pub(in crate::canonical_scene::store) const fn output_attribute_count(self) -> u32 {
        self.output_attributes
    }

    pub(in crate::canonical_scene::store) const fn normal_attribute(self) -> u32 {
        self.normal_attribute
    }

    pub(in crate::canonical_scene::store) const fn source_normal(self) -> Option<u32> {
        self.source_normal
    }

    pub(in crate::canonical_scene::store) const fn source_tangent(self) -> Option<u32> {
        self.source_tangent
    }

    pub(in crate::canonical_scene::store) const fn source_position(self) -> u32 {
        self.source_position
    }

    pub(in crate::canonical_scene::store) const fn source_smoothing_group(self) -> Option<u32> {
        self.source_smoothing_group
    }

    /// Global attribute mapping, including a replaced Normal and excluding a removed Tangent.
    pub(in crate::canonical_scene::store) fn output_attribute(self, source: u32) -> Option<u32> {
        if source >= self.source_attributes || self.source_tangent == Some(source) {
            return None;
        }
        source
            .checked_sub(u32::from(
                self.source_tangent.is_some_and(|index| index < source),
            ))?
            .checked_add(u32::from(
                self.source_normal.is_none() && source >= self.insertion_boundary,
            ))
    }

    /// A newly added Normal has no source row; a replacement maps to the old Normal row.
    pub(in crate::canonical_scene::store) fn source_attribute(self, output: u32) -> Option<u32> {
        if output >= self.output_attributes {
            return None;
        }
        if output == self.normal_attribute {
            return self.source_normal;
        }
        let without_addition = output.checked_sub(u32::from(
            self.source_normal.is_none() && output > self.normal_attribute,
        ))?;
        without_addition.checked_add(u32::from(
            self.source_tangent
                .is_some_and(|index| index <= without_addition),
        ))
    }
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct GeometryNormalsPlan<'a> {
    fixture: TestSelection<'a>,
    mapping: NormalAttributeMapping,
}
impl<'a> GeometryNormalsPlan<'a> {
    pub(in crate::canonical_scene::store) fn try_new<C: TypedStorageBuildControl>(
        source: &'a GeometryStorage,
        target: usize,
        normals: VectorValues<'a, 3>,
        write: NormalWritePolicy,
        tangents: TangentPolicy,
        control: &mut C,
    ) -> EditResult<Self, C::Error> {
        let primitive = source
            .primitives
            .get(target)
            .ok_or(TypedStorageBuildError::Invariant)?;
        if primitive.topology != TopologyKind::TriangleList || primitive.corner_count == 0 {
            return Err(TypedStorageBuildError::Invariant);
        }
        let attributes = primitive
            .attributes
            .checked(source.attributes.len())
            .ok_or(TypedStorageBuildError::Invariant)?;
        let normal = source.attributes[attributes.clone()]
            .iter()
            .any(|a| a.semantic == StoredAttributeSemantic::Normal);
        let tangent = source.attributes[attributes.clone()]
            .iter()
            .any(|a| a.semantic == StoredAttributeSemantic::Tangent);
        if normal != (write == NormalWritePolicy::Replace)
            || tangent && tangents == TangentPolicy::Reject
        {
            return Err(TypedStorageBuildError::Invariant);
        }
        let mut fixture = TestSelection::description(source, target);
        fixture.normals = Some(normals);
        fixture.remove_tangents = tangents == TangentPolicy::Remove;
        let fixture = fixture.seal(control)?;
        let m = fixture.mapping;
        let insertion_boundary = attributes
            .clone()
            .find(|&i| {
                !matches!(
                    source.attributes[i].semantic,
                    StoredAttributeSemantic::Position | StoredAttributeSemantic::Normal
                )
            })
            .unwrap_or(attributes.end) as u32;
        let mapping = NormalAttributeMapping {
            source_attributes: source.attributes.len() as u32,
            output_attributes: fixture.capacities[2].desired_capacity() as u32,
            insertion_boundary,
            normal_attribute: m.output_normal.ok_or(TypedStorageBuildError::Invariant)?,
            source_normal: m.normal,
            source_tangent: m.tangent.filter(|_| fixture.remove_tangents),
            source_position: m.position.ok_or(TypedStorageBuildError::Invariant)?,
            source_smoothing_group: m.smoothing,
        };
        Ok(Self { fixture, mapping })
    }
    pub(in crate::canonical_scene::store) const fn target_row(&self) -> usize {
        self.fixture.target
    }

    pub(in crate::canonical_scene::store) const fn capacities(&self) -> [CapacityPlan; 9] {
        self.fixture.capacities
    }

    /// One action per output row/scalar and one string action per name byte; no lookup actions.
    pub(in crate::canonical_scene::store) const fn work(&self) -> u64 {
        self.fixture.work
    }

    pub(in crate::canonical_scene::store) const fn validation_work(&self) -> u64 {
        self.fixture.validation_work
    }

    /// Each descriptor, referenced name byte and scalar is compared exactly once on success.
    pub(in crate::canonical_scene::store) const fn correspondence_work(&self) -> u64 {
        self.fixture.work
    }

    pub(in crate::canonical_scene::store) const fn name_bytes(&self) -> usize {
        self.fixture.capacities[4].desired_capacity()
    }

    pub(in crate::canonical_scene::store) const fn attribute_mapping(
        &self,
    ) -> NormalAttributeMapping {
        self.mapping
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) const fn source_attribute_count(&self) -> u32 {
        self.mapping.source_attribute_count()
    }

    pub(in crate::canonical_scene::store) const fn output_attribute_count(&self) -> u32 {
        self.mapping.output_attribute_count()
    }

    pub(in crate::canonical_scene::store) const fn normal_attribute(&self) -> u32 {
        self.mapping.normal_attribute()
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) const fn source_normal(&self) -> Option<u32> {
        self.mapping.source_normal()
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) const fn source_tangent(&self) -> Option<u32> {
        self.mapping.source_tangent()
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) const fn source_position(&self) -> u32 {
        self.mapping.source_position()
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) const fn source_smoothing_group(&self) -> Option<u32> {
        self.mapping.source_smoothing_group()
    }

    pub(in crate::canonical_scene::store) fn output_corner_count(&self) -> u32 {
        // try_new established the u32 cardinality and binds this immutable slice.
        self.fixture.normals.expect("normal fixture").len() as u32
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn output_face_count(&self) -> u32 {
        self.output_corner_count() / 3
    }

    /// Global attribute mapping, including the replaced Normal and excluding a removed Tangent.
    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn output_attribute(&self, source: u32) -> Option<u32> {
        self.mapping.output_attribute(source)
    }

    /// A newly added Normal has no source row; a replacement maps to the old Normal row.
    pub(in crate::canonical_scene::store) fn source_attribute(&self, output: u32) -> Option<u32> {
        self.mapping.source_attribute(output)
    }

    pub(in crate::canonical_scene::store) fn materialize<C: TypedStorageBuildControl>(
        &self,
        control: &mut C,
    ) -> EditResult<GeometryStorage, C::Error> {
        self.fixture.materialize(control)
    }
    pub(in crate::canonical_scene::store) fn validate_correspondence<
        C: TypedStorageValidationControl,
    >(
        &self,
        actual: &GeometryStorage,
        control: &mut C,
    ) -> Result<bool, C::Error> {
        self.fixture.validate_correspondence(actual, control)
    }
}
#[cfg(test)]
mod tests;
