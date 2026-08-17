use crate::canonical_scene::store::validation::NodeSceneValidationProof;
use crate::geometry::{PrimitiveCardinality, TopologyKind};
use crate::identity::{LogicalMeshId, PrimitiveId, SceneGenerationId};

use super::super::{NodeSceneSemanticResult, invariant};
use super::{
    NodeSceneGeometryAttributeSemantic, NodeSceneGeometryCursor, NodeSceneGeometryTables,
    SceneCanonicalStore, advance_row,
};

/// Proof-qualified primitive facts needed by read-only post-process planning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneGeometryPrimitiveFacts {
    id: PrimitiveId,
    topology: TopologyKind,
    cardinality: PrimitiveCardinality,
    has_normals: bool,
}

impl NodeSceneGeometryPrimitiveFacts {
    pub(in crate::canonical_scene::store) const fn id(self) -> PrimitiveId {
        self.id
    }

    pub(in crate::canonical_scene::store) const fn topology(self) -> TopologyKind {
        self.topology
    }

    pub(in crate::canonical_scene::store) const fn cardinality(self) -> PrimitiveCardinality {
        self.cardinality
    }

    pub(in crate::canonical_scene::store) const fn has_normals(self) -> bool {
        self.has_normals
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PrimitiveOwnerRange {
    logical_mesh: LogicalMeshId,
    offset: usize,
    count: usize,
}

impl PrimitiveOwnerRange {
    fn contains(self, primitive: usize) -> bool {
        primitive
            .checked_sub(self.offset)
            .is_some_and(|local| local < self.count)
    }

    fn id(self, primitive: usize) -> NodeSceneSemanticResult<PrimitiveId> {
        let local = primitive
            .checked_sub(self.offset)
            .filter(|local| *local < self.count)
            .ok_or_else(|| invariant("geometry inspection primitive is outside its owner range"))?;
        let local = u32::try_from(local)
            .map_err(|_| invariant("geometry inspection primitive local index overflows"))?;
        Ok(PrimitiveId::new(self.logical_mesh, local))
    }
}

/// Allocation-free primitive inspection without exposing geometry tables or payload leaves.
#[must_use = "the geometry inspection cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneGeometryInspectionCursor<'scene> {
    logical_meshes: NodeSceneGeometryCursor<'scene>,
    primitives: NodeSceneGeometryCursor<'scene>,
    attributes: NodeSceneGeometryCursor<'scene>,
    generation: SceneGenerationId,
    primitive_owner: Option<PrimitiveOwnerRange>,
    primitive_count: usize,
    attribute_count: usize,
    next_primitive: usize,
    next_attribute: usize,
}

impl<'scene> NodeSceneGeometryInspectionCursor<'scene> {
    pub(in crate::canonical_scene::store) fn try_new(
        canonical: &'scene SceneCanonicalStore,
        proof: NodeSceneValidationProof,
        generation: SceneGenerationId,
    ) -> NodeSceneSemanticResult<Self> {
        let logical_meshes = NodeSceneGeometryTables::try_new(canonical, proof)?.into_cursor(proof);
        let mut primitives = NodeSceneGeometryTables::try_new(canonical, proof)?.into_cursor(proof);
        while primitives.next_logical_mesh()?.is_some() {}

        let mut attributes = NodeSceneGeometryTables::try_new(canonical, proof)?.into_cursor(proof);
        while attributes.next_logical_mesh()?.is_some() {}
        while attributes.next_primitive()?.is_some() {}

        Ok(Self {
            logical_meshes,
            primitives,
            attributes,
            generation,
            primitive_owner: None,
            primitive_count: proof.primitive_count(),
            attribute_count: proof.attribute_count(),
            next_primitive: 0,
            next_attribute: 0,
        })
    }

    pub(in crate::canonical_scene::store) const fn primitive_count(&self) -> usize {
        self.primitive_count
    }

    pub(in crate::canonical_scene::store) fn next_primitive(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneGeometryPrimitiveFacts>> {
        if self.next_primitive == self.primitive_count {
            return Ok(None);
        }
        if self.next_primitive > self.primitive_count {
            return Err(invariant(
                "geometry inspection advanced beyond its proof-qualified primitive count",
            ));
        }

        let primitive = self.primitives.next_primitive()?.ok_or_else(|| {
            invariant("geometry inspection ended before its proof-qualified primitive count")
        })?;
        if primitive.source_index() != self.next_primitive {
            return Err(invariant(
                "geometry inspection primitive order disagrees with its proof-qualified row",
            ));
        }
        if primitive.attribute_offset() != self.next_attribute {
            return Err(invariant(
                "geometry inspection primitive attribute ranges are not contiguous",
            ));
        }

        let id = self.primitive_id(self.next_primitive)?;

        let mut has_normals = false;
        for _ in 0..primitive.attribute_count() {
            let attribute = self.attributes.next_attribute()?.ok_or_else(|| {
                invariant("geometry inspection ended before a primitive attribute range")
            })?;
            if attribute.source_index() != self.next_attribute {
                return Err(invariant(
                    "geometry inspection attribute order disagrees with its proof-qualified row",
                ));
            }
            has_normals |= matches!(
                attribute.semantic(),
                NodeSceneGeometryAttributeSemantic::Normal
            );
            self.next_attribute = advance_row(
                self.next_attribute,
                "geometry inspection attribute index overflows",
            )?;
            let _payload = attribute.into_payload();
        }

        self.next_primitive = advance_row(
            self.next_primitive,
            "geometry inspection primitive index overflows",
        )?;
        Ok(Some(NodeSceneGeometryPrimitiveFacts {
            id,
            topology: primitive.topology(),
            cardinality: PrimitiveCardinality::from_validated_counts(
                primitive.vertex_count(),
                primitive.corner_count(),
                primitive.face_count(),
            ),
            has_normals,
        }))
    }

    fn primitive_id(&mut self, primitive: usize) -> NodeSceneSemanticResult<PrimitiveId> {
        while !self
            .primitive_owner
            .is_some_and(|owner| owner.contains(primitive))
        {
            let logical_mesh = self.logical_meshes.next_logical_mesh()?.ok_or_else(|| {
                invariant("geometry inspection could not qualify a primitive owner")
            })?;
            let index = u32::try_from(logical_mesh.source_index())
                .map_err(|_| invariant("geometry inspection logical-mesh index overflows"))?;
            self.primitive_owner = Some(PrimitiveOwnerRange {
                logical_mesh: LogicalMeshId::new(self.generation, index),
                offset: logical_mesh.primitive_offset(),
                count: logical_mesh.primitive_count(),
            });
        }
        self.primitive_owner
            .ok_or_else(|| invariant("geometry inspection lost its primitive owner"))?
            .id(primitive)
    }

    pub(in crate::canonical_scene::store) fn finish(mut self) -> NodeSceneSemanticResult<()> {
        if self.next_primitive != self.primitive_count {
            return Err(invariant(
                "geometry inspection ended before its proof-qualified primitive count",
            ));
        }
        if self.next_attribute != self.attribute_count {
            return Err(invariant(
                "geometry inspection ended before its proof-qualified attribute count",
            ));
        }
        if self.primitives.next_primitive()?.is_some() {
            return Err(invariant(
                "geometry inspection retained an unexpected primitive after exhaustion",
            ));
        }
        if self.attributes.next_attribute()?.is_some() {
            return Err(invariant(
                "geometry inspection retained an unexpected attribute after exhaustion",
            ));
        }
        while self.logical_meshes.next_logical_mesh()?.is_some() {}
        while self.attributes.next_mesh_instance()?.is_some() {}
        self.attributes.finish()
    }
}
