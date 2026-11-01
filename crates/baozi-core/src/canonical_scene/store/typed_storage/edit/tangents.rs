//! Test fixture headers over the sole production selected reconstruction engine.
use super::super::validation::TypedStorageValidationControl;
use super::super::{
    StoredAttributeSemantic, StoredTangentBasis, StoredTangentConventions, StoredTangentEncoding,
};
use super::test_selection::TestSelection;
use super::{
    EditResult, GeometryStorage, StoredAttribute, StoredAttributePayload, StoredLogicalMesh,
    StoredMeshInstance, StoredPrimitive, TableRange, TypedStorageBuildControl,
    TypedStorageBuildError, VectorValues, same_range,
};
use crate::attribute::{
    AttributeDomain, TangentBasisConvention, TangentConventions, TangentHandednessEncoding,
};
use crate::geometry::TopologyKind;
use baozi_operation::CapacityPlan;

/// Allocation-free global owner correspondence, including a replaced tangent's source owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct TangentAttributeMapping {
    source_attributes: u32,
    output_attributes: u32,
    tangent_attribute: u32,
    source_tangent: Option<u32>,
    source_position: u32,
    source_normal: u32,
    source_uv: u32,
}

impl TangentAttributeMapping {
    pub(in crate::canonical_scene::store) const fn tangent_attribute(self) -> u32 {
        self.tangent_attribute
    }

    pub(in crate::canonical_scene::store) const fn source_tangent(self) -> Option<u32> {
        self.source_tangent
    }

    pub(in crate::canonical_scene::store) const fn source_position(self) -> u32 {
        self.source_position
    }

    pub(in crate::canonical_scene::store) const fn source_normal(self) -> u32 {
        self.source_normal
    }

    pub(in crate::canonical_scene::store) const fn source_uv(self) -> u32 {
        self.source_uv
    }

    pub(in crate::canonical_scene::store) fn output_attribute(self, source: u32) -> Option<u32> {
        if source >= self.source_attributes {
            return None;
        }
        source.checked_add(u32::from(
            self.source_tangent.is_none() && source >= self.tangent_attribute,
        ))
    }

    pub(in crate::canonical_scene::store) fn source_attribute(self, output: u32) -> Option<u32> {
        if output >= self.output_attributes {
            return None;
        }
        if output == self.tangent_attribute {
            return self.source_tangent;
        }
        output.checked_sub(u32::from(
            self.source_tangent.is_none() && output > self.tangent_attribute,
        ))
    }
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct GeometryTangentsPlan<'a> {
    fixture: TestSelection<'a>,
    mapping: TangentAttributeMapping,
}
impl<'a> GeometryTangentsPlan<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::canonical_scene::store) fn try_new<C: TypedStorageBuildControl>(
        source: &'a GeometryStorage,
        target: usize,
        tangents: VectorValues<'a, 4>,
        had_tangent: bool,
        normal_domain: AttributeDomain,
        uv_set: u32,
        uv_domain: AttributeDomain,
        conventions: &TangentConventions,
        control: &mut C,
    ) -> EditResult<Self, C::Error> {
        let primitive = source
            .primitives
            .get(target)
            .ok_or(TypedStorageBuildError::Invariant)?;
        if primitive.topology != TopologyKind::TriangleList
            || primitive.corner_count == 0
            || !matches!(
                normal_domain,
                AttributeDomain::Vertex | AttributeDomain::Corner
            )
            || !matches!(uv_domain, AttributeDomain::Vertex | AttributeDomain::Corner)
            || !matches!(
                conventions.basis,
                TangentBasisConvention::TriangleUvDerivative
            )
            || !matches!(conventions.encoding, TangentHandednessEncoding::SignInW)
        {
            return Err(TypedStorageBuildError::Invariant);
        }
        let attributes = primitive
            .attributes
            .get(&source.attributes)
            .ok_or(TypedStorageBuildError::Invariant)?;
        if attributes
            .iter()
            .any(|a| a.semantic == StoredAttributeSemantic::Tangent)
            != had_tangent
        {
            return Err(TypedStorageBuildError::Invariant);
        }
        let normal = attributes
            .iter()
            .position(|a| {
                a.semantic == StoredAttributeSemantic::Normal && a.domain == normal_domain
            })
            .map(|i| i + primitive.attributes.start())
            .ok_or(TypedStorageBuildError::Invariant)?;
        let uv = attributes
            .iter()
            .find(|a| {
                a.semantic == StoredAttributeSemantic::Texcoord(uv_set)
                    && a.domain == uv_domain
                    && a.lanes == 2
            })
            .ok_or(TypedStorageBuildError::Invariant)?;
        if !matches!(
            uv.payload,
            StoredAttributePayload::DenseF32(_)
                | StoredAttributePayload::DenseF64(_)
                | StoredAttributePayload::SparseF32 { .. }
                | StoredAttributePayload::SparseF64 { .. }
        ) {
            return Err(TypedStorageBuildError::Invariant);
        }
        let mut fixture = TestSelection::description(source, target);
        fixture.tangents = Some(tangents);
        fixture.normal_domain = normal_domain;
        fixture.uv_set = uv_set;
        fixture.uv_domain = uv_domain;
        let fixture = fixture.seal(control)?;
        let m = fixture.mapping;
        let mapping = TangentAttributeMapping {
            source_attributes: source.attributes.len() as u32,
            output_attributes: fixture.capacities[2].desired_capacity() as u32,
            tangent_attribute: m.output_tangent.ok_or(TypedStorageBuildError::Invariant)?,
            source_tangent: m.tangent,
            source_position: m.position.ok_or(TypedStorageBuildError::Invariant)?,
            source_normal: normal as u32,
            source_uv: m.uv.ok_or(TypedStorageBuildError::Invariant)?,
        };
        Ok(Self { fixture, mapping })
    }
    pub(in crate::canonical_scene::store) const fn target_row(&self) -> usize {
        self.fixture.target
    }

    pub(in crate::canonical_scene::store) const fn capacities(&self) -> [CapacityPlan; 9] {
        self.fixture.capacities
    }

    pub(in crate::canonical_scene::store) const fn work(&self) -> u64 {
        self.fixture.work
    }

    pub(in crate::canonical_scene::store) const fn validation_work(&self) -> u64 {
        self.fixture.validation_work
    }

    pub(in crate::canonical_scene::store) const fn correspondence_work(&self) -> u64 {
        self.fixture.work
    }

    pub(in crate::canonical_scene::store) const fn name_bytes(&self) -> usize {
        self.fixture.capacities[4].desired_capacity()
    }

    pub(in crate::canonical_scene::store) const fn attribute_mapping(
        &self,
    ) -> TangentAttributeMapping {
        self.mapping
    }

    pub(in crate::canonical_scene::store) const fn output_attribute_count(&self) -> u32 {
        self.mapping.output_attributes
    }

    pub(in crate::canonical_scene::store) fn output_corner_count(&self) -> u32 {
        self.fixture.tangents.expect("tangent fixture").len() as u32
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
#[path = "tangents_tests.rs"]
mod tests;
