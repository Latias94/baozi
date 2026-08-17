use crate::deform::{MorphDeltaSemantic, SkinMatrixSpaces};
use crate::entity::EntityName;
use crate::math::Mat4;

use super::Presence;
use super::geometry::AuthoringRange;

/// Identity of one skin within an authoring batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SkinLocalId(u32);

impl SkinLocalId {
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    pub(in crate::canonical_scene::store) const fn index(self) -> u32 {
        self.0
    }
}

/// Producer-stable identity of one logical-mesh-owned morph target.
///
/// This identity is independent of the target's current canonical ordinal. The
/// identities owned by one logical mesh form a permutation of `0..target_count`.
/// A format adapter derives that bounded ordinal from its typed source-object
/// locator; for glTF the source target ordinal is the stable identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MorphTargetIdentity(u32);

impl MorphTargetIdentity {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub(in crate::canonical_scene::store) const fn value(self) -> u32 {
        self.0
    }
}

/// One palette-local influence preserving source tie order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InfluenceAuthoring {
    joint: u32,
    weight: f32,
    source_order: u32,
}

impl InfluenceAuthoring {
    pub const fn new(joint: u32, weight: f32, source_order: u32) -> Self {
        Self {
            joint,
            weight,
            source_order,
        }
    }

    pub(in crate::canonical_scene::store) const fn joint(self) -> u32 {
        self.joint
    }

    pub(in crate::canonical_scene::store) const fn weight(self) -> f32 {
        self.weight
    }

    pub(in crate::canonical_scene::store) const fn source_order(self) -> u32 {
        self.source_order
    }
}

/// Variable-length compressed influence rows for one primitive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InfluenceRowsAuthoring<'a> {
    vertex_offsets: &'a [u32],
    influences: &'a [InfluenceAuthoring],
}

impl<'a> InfluenceRowsAuthoring<'a> {
    pub const fn new(vertex_offsets: &'a [u32], influences: &'a [InfluenceAuthoring]) -> Self {
        Self {
            vertex_offsets,
            influences,
        }
    }

    pub(in crate::canonical_scene::store) const fn vertex_offsets(self) -> &'a [u32] {
        self.vertex_offsets
    }

    pub(in crate::canonical_scene::store) const fn influences(self) -> &'a [InfluenceAuthoring] {
        self.influences
    }
}

/// Dense or sparse relative morph deltas over a primitive vertex domain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MorphDeltaPayloadAuthoring<'a> {
    /// One tightly packed F32x3 value for every primitive vertex.
    Dense { values: &'a [f32] },
    /// Strictly increasing vertex indices with one tightly packed F32x3 value each.
    Sparse {
        logical_count: u32,
        indices: &'a [u32],
        values: &'a [f32],
    },
}

/// One position, normal, or tangent delta stream.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MorphDeltaAuthoring<'a> {
    semantic: MorphDeltaSemantic,
    payload: MorphDeltaPayloadAuthoring<'a>,
}

impl<'a> MorphDeltaAuthoring<'a> {
    pub const fn new(
        semantic: MorphDeltaSemantic,
        payload: MorphDeltaPayloadAuthoring<'a>,
    ) -> Self {
        Self { semantic, payload }
    }

    pub(in crate::canonical_scene::store) const fn semantic(self) -> MorphDeltaSemantic {
        self.semantic
    }

    pub(in crate::canonical_scene::store) const fn payload(self) -> MorphDeltaPayloadAuthoring<'a> {
        self.payload
    }
}

/// One primitive-local projection of a logical morph target.
///
/// `deltas` must be in canonical Position, Normal, Tangent order. The target's
/// logical ordinal is its position in the owning primitive's target slice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrimitiveMorphTargetAuthoring<'a> {
    deltas: &'a [MorphDeltaAuthoring<'a>],
}

impl<'a> PrimitiveMorphTargetAuthoring<'a> {
    pub const fn new(deltas: &'a [MorphDeltaAuthoring<'a>]) -> Self {
        Self { deltas }
    }

    pub(in crate::canonical_scene::store) const fn deltas(self) -> &'a [MorphDeltaAuthoring<'a>] {
        self.deltas
    }
}

/// Deformation payload owned by one primitive in geometry-table order.
///
/// Influence rows preserve palette-local joint indices. A mesh instance supplies
/// the skin palette used to interpret those indices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrimitiveDeformationAuthoring<'a> {
    influences: Presence<InfluenceRowsAuthoring<'a>>,
    morph_targets: &'a [PrimitiveMorphTargetAuthoring<'a>],
}

impl<'a> PrimitiveDeformationAuthoring<'a> {
    pub const fn new(
        influences: Presence<InfluenceRowsAuthoring<'a>>,
        morph_targets: &'a [PrimitiveMorphTargetAuthoring<'a>],
    ) -> Self {
        Self {
            influences,
            morph_targets,
        }
    }

    pub(in crate::canonical_scene::store) const fn influences(
        self,
    ) -> Presence<InfluenceRowsAuthoring<'a>> {
        self.influences
    }

    pub(in crate::canonical_scene::store) const fn morph_targets(
        self,
    ) -> &'a [PrimitiveMorphTargetAuthoring<'a>] {
        self.morph_targets
    }
}

/// Stable identity and optional name for one logical morph target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MorphTargetDefinitionAuthoring<'a> {
    identity: MorphTargetIdentity,
    name: Presence<&'a EntityName>,
}

impl<'a> MorphTargetDefinitionAuthoring<'a> {
    pub const fn new(identity: MorphTargetIdentity, name: Presence<&'a EntityName>) -> Self {
        Self { identity, name }
    }

    pub(in crate::canonical_scene::store) const fn identity(self) -> MorphTargetIdentity {
        self.identity
    }

    pub(in crate::canonical_scene::store) const fn name(self) -> Presence<&'a EntityName> {
        self.name
    }
}

/// Morph definitions and defaults owned by one logical mesh in geometry-table order.
///
/// `definitions` names one contiguous range in the batch-level definition table.
/// Its count must equal the default-weight width. Every primitive of the logical
/// mesh must provide exactly that many target projections.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogicalMorphAuthoring<'a> {
    definitions: AuthoringRange,
    default_weights: &'a [f32],
}

impl<'a> LogicalMorphAuthoring<'a> {
    pub const fn new(definitions: AuthoringRange, default_weights: &'a [f32]) -> Self {
        Self {
            definitions,
            default_weights,
        }
    }

    pub(in crate::canonical_scene::store) const fn definitions(self) -> AuthoringRange {
        self.definitions
    }

    pub(in crate::canonical_scene::store) const fn default_weights(self) -> &'a [f32] {
        self.default_weights
    }
}

/// Skin selection and optional complete morph override for one mesh instance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshInstanceDeformationAuthoring<'a> {
    skin: Presence<SkinLocalId>,
    morph_weights: Presence<&'a [f32]>,
}

impl<'a> MeshInstanceDeformationAuthoring<'a> {
    pub const fn new(skin: Presence<SkinLocalId>, morph_weights: Presence<&'a [f32]>) -> Self {
        Self {
            skin,
            morph_weights,
        }
    }

    pub(in crate::canonical_scene::store) const fn skin(self) -> Presence<SkinLocalId> {
        self.skin
    }

    pub(in crate::canonical_scene::store) const fn morph_weights(self) -> Presence<&'a [f32]> {
        self.morph_weights
    }
}

/// One ordered joint palette with explicit canonical matrix spaces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkinAuthoring<'a> {
    name: Presence<&'a EntityName>,
    joints: &'a [super::NodeLocalId],
    inverse_bind_matrices: &'a [Mat4],
    skeleton_root: Presence<super::NodeLocalId>,
    matrix_spaces: SkinMatrixSpaces,
}

impl<'a> SkinAuthoring<'a> {
    pub const fn new(
        name: Presence<&'a EntityName>,
        joints: &'a [super::NodeLocalId],
        inverse_bind_matrices: &'a [Mat4],
        skeleton_root: Presence<super::NodeLocalId>,
        matrix_spaces: SkinMatrixSpaces,
    ) -> Self {
        Self {
            name,
            joints,
            inverse_bind_matrices,
            skeleton_root,
            matrix_spaces,
        }
    }

    pub(in crate::canonical_scene::store) const fn name(self) -> Presence<&'a EntityName> {
        self.name
    }

    pub(in crate::canonical_scene::store) const fn joints(self) -> &'a [super::NodeLocalId] {
        self.joints
    }

    pub(in crate::canonical_scene::store) const fn inverse_bind_matrices(self) -> &'a [Mat4] {
        self.inverse_bind_matrices
    }

    pub(in crate::canonical_scene::store) const fn skeleton_root(
        self,
    ) -> Presence<super::NodeLocalId> {
        self.skeleton_root
    }

    pub(in crate::canonical_scene::store) const fn matrix_spaces(self) -> SkinMatrixSpaces {
        self.matrix_spaces
    }
}

/// Borrowed deformation-domain input aligned with geometry table order.
///
/// `logical_meshes`, `primitives`, and `mesh_instances` have exact one-to-one
/// cardinality with the corresponding validated geometry tables. Skins are a
/// separate scene-level table addressed by [`SkinLocalId`].
#[derive(Debug, Clone, Copy)]
pub struct DeformationAuthoringBatch<'a> {
    skins: &'a [SkinAuthoring<'a>],
    morph_target_definitions: &'a [MorphTargetDefinitionAuthoring<'a>],
    logical_meshes: &'a [LogicalMorphAuthoring<'a>],
    primitives: &'a [PrimitiveDeformationAuthoring<'a>],
    mesh_instances: &'a [MeshInstanceDeformationAuthoring<'a>],
}

impl<'a> DeformationAuthoringBatch<'a> {
    /// Creates a batch for a Scene whose geometry-aligned tables are also empty.
    ///
    /// Geometry with logical meshes, primitives, or mesh instances still requires
    /// one explicit deformation row per corresponding geometry row, even when
    /// every deformation payload is absent.
    pub const fn empty() -> Self {
        Self::new(&[], &[], &[], &[], &[])
    }

    pub const fn new(
        skins: &'a [SkinAuthoring<'a>],
        morph_target_definitions: &'a [MorphTargetDefinitionAuthoring<'a>],
        logical_meshes: &'a [LogicalMorphAuthoring<'a>],
        primitives: &'a [PrimitiveDeformationAuthoring<'a>],
        mesh_instances: &'a [MeshInstanceDeformationAuthoring<'a>],
    ) -> Self {
        Self {
            skins,
            morph_target_definitions,
            logical_meshes,
            primitives,
            mesh_instances,
        }
    }

    pub(in crate::canonical_scene::store) const fn skins(self) -> &'a [SkinAuthoring<'a>] {
        self.skins
    }

    pub(in crate::canonical_scene::store) const fn morph_target_definitions(
        self,
    ) -> &'a [MorphTargetDefinitionAuthoring<'a>] {
        self.morph_target_definitions
    }

    pub(in crate::canonical_scene::store) const fn logical_meshes(
        self,
    ) -> &'a [LogicalMorphAuthoring<'a>] {
        self.logical_meshes
    }

    pub(in crate::canonical_scene::store) const fn primitives(
        self,
    ) -> &'a [PrimitiveDeformationAuthoring<'a>] {
        self.primitives
    }

    pub(in crate::canonical_scene::store) const fn mesh_instances(
        self,
    ) -> &'a [MeshInstanceDeformationAuthoring<'a>] {
        self.mesh_instances
    }
}
