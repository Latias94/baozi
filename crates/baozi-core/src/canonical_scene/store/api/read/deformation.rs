//! Borrowed skin palettes, variable influences, and logical morph projections.

use std::fmt;

use crate::canonical_scene::store::construction::node_scene::semantic::deformation::{
    NodeSceneDeformationMeshInstance, NodeSceneDeformationMorphDefinition,
    NodeSceneDeformationMorphDelta, NodeSceneDeformationMorphPayload,
    NodeSceneDeformationMorphTarget, NodeSceneDeformationSkin, NodeSceneDeformationTables,
};
use crate::deform::{MorphDeltaSemantic, SkinMatrixSpaces};
use crate::identity::{MorphTargetId, NodeId, SceneEntityKind, SceneLookupError, SkinId};
use crate::math::{Mat4, Vec3};

use super::{
    LogicalMeshRef, MeshInstanceRef, NodeRef, PrimitiveRef, ValidatedScene, checked_index,
    expect_validated,
};

fn tables(scene: &ValidatedScene) -> NodeSceneDeformationTables<'_> {
    let owner = scene.as_node_scene();
    expect_validated(NodeSceneDeformationTables::try_new(
        owner.read_canonical(),
        owner.read_proof(),
    ))
}

/// An ordered joint palette and its bind matrices borrowed from a validated Scene.
#[derive(Clone, Copy)]
pub struct SkinRef<'s> {
    scene: &'s ValidatedScene,
    index: usize,
}

impl<'s> SkinRef<'s> {
    pub(super) const fn at(scene: &'s ValidatedScene, index: usize) -> Self {
        Self { scene, index }
    }

    /// Returns this skin's generation-qualified identity.
    pub fn id(self) -> SkinId {
        SkinId::new(self.scene.generation(), self.index as u32)
    }

    /// Preserves an absent name and an authored empty name.
    pub fn name(self) -> Option<&'s str> {
        self.read().name()
    }

    /// Returns the optional authored skeleton root.
    pub fn skeleton_root(self) -> Option<NodeRef<'s>> {
        self.read()
            .skeleton_root()
            .map(|index| NodeRef::at(self.scene, index))
    }

    pub fn joint_count(self) -> usize {
        self.read()
            .take_joints()
            .expect("validated skin has a joint palette")
            .declared_count()
    }

    /// Iterates generation-qualified joint NodeIds in palette order.
    ///
    /// Influence joint indices address this palette through the mesh instance's skin.
    pub fn joints(
        self,
    ) -> impl ExactSizeIterator<Item = NodeId> + DoubleEndedIterator + Clone + 's {
        self.read()
            .take_joints()
            .expect("validated skin has a joint palette")
            .into_slice()
            .iter()
            .map(move |canonical| NodeId::new(self.scene.generation(), *canonical - 1))
    }

    /// Iterates column-major inverse-bind matrices in joint palette order.
    ///
    /// Only the requested matrix is copied into a `Mat4`; the complete payload stays borrowed.
    pub fn inverse_bind_matrices(
        self,
    ) -> impl ExactSizeIterator<Item = Mat4> + DoubleEndedIterator + Clone + 's {
        self.read()
            .take_inverse_bind_matrices()
            .expect("validated skin has inverse-bind matrices")
            .into_matrices()
            .map(expect_validated)
    }

    /// Returns the explicit matrix-space contract, including any source correction.
    pub fn matrix_spaces(self) -> SkinMatrixSpaces {
        self.read().matrix_spaces()
    }

    fn read(self) -> NodeSceneDeformationSkin<'s> {
        expect_validated(
            tables(self.scene).skin_at(self.scene.as_node_scene().read_proof(), self.index),
        )
    }
}

impl fmt::Debug for SkinRef<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SkinRef")
            .field("id", &self.id())
            .finish_non_exhaustive()
    }
}

/// Variable-length influence rows over a primitive's vertex domain.
///
/// Rows and their parallel scalar slices borrow the Scene. Reading a row uses two offsets,
/// without scanning other vertices or normalizing, sorting, or truncating influences.
#[derive(Debug, Clone, Copy)]
pub struct InfluenceRows<'s> {
    offsets: &'s [u32],
    joints: &'s [u32],
    weights: &'s [f32],
    source_orders: &'s [u32],
}

impl<'s> InfluenceRows<'s> {
    /// Returns the primitive vertex count, including vertices with empty rows.
    pub const fn len(self) -> usize {
        self.offsets.len() - 1
    }

    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// Returns one vertex's row in constant time, preserving an authored empty row.
    pub fn get(self, vertex: usize) -> Option<InfluenceRowRef<'s>> {
        let start = *self.offsets.get(vertex)? as usize;
        let end = *self.offsets.get(vertex.checked_add(1)?)? as usize;
        Some(InfluenceRowRef {
            vertex: vertex as u32,
            joints: &self.joints[start..end],
            weights: &self.weights[start..end],
            source_orders: &self.source_orders[start..end],
        })
    }

    /// Iterates rows in vertex order. Independent iterators may be consumed in any order.
    pub fn iter(
        self,
    ) -> impl ExactSizeIterator<Item = InfluenceRowRef<'s>> + DoubleEndedIterator + Clone + 's {
        (0..self.len())
            .map(move |vertex| self.get(vertex).expect("validated influence vertex exists"))
    }
}

/// One vertex's parallel influence slices, preserving the retained order and source tie order.
#[derive(Debug, Clone, Copy)]
pub struct InfluenceRowRef<'s> {
    vertex: u32,
    joints: &'s [u32],
    weights: &'s [f32],
    source_orders: &'s [u32],
}

impl<'s> InfluenceRowRef<'s> {
    pub const fn vertex(self) -> u32 {
        self.vertex
    }

    pub const fn len(self) -> usize {
        self.joints.len()
    }

    pub const fn is_empty(self) -> bool {
        self.joints.is_empty()
    }

    /// Palette-local joint indices; the instance's skin determines the joint NodeIds.
    pub const fn joint_indices(self) -> &'s [u32] {
        self.joints
    }

    /// Retained weights, without normalization or precision conversion.
    pub const fn weights(self) -> &'s [f32] {
        self.weights
    }

    pub const fn source_orders(self) -> &'s [u32] {
        self.source_orders
    }

    pub fn get(self, index: usize) -> Option<InfluenceRef<'s>> {
        Some(InfluenceRef {
            joint: self.joints.get(index)?,
            weight: self.weights.get(index)?,
            source_order: self.source_orders.get(index)?,
        })
    }

    pub fn iter(
        self,
    ) -> impl ExactSizeIterator<Item = InfluenceRef<'s>> + DoubleEndedIterator + Clone + 's {
        self.joints
            .iter()
            .zip(self.weights)
            .zip(self.source_orders)
            .map(|((joint, weight), source_order)| InfluenceRef {
                joint,
                weight,
                source_order,
            })
    }
}

/// One palette-local influence borrowing the Scene's parallel scalar payloads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InfluenceRef<'s> {
    joint: &'s u32,
    weight: &'s f32,
    source_order: &'s u32,
}

impl InfluenceRef<'_> {
    pub const fn palette_joint(self) -> u32 {
        *self.joint
    }

    pub const fn weight(self) -> f32 {
        *self.weight
    }

    pub const fn source_order(self) -> u32 {
        *self.source_order
    }
}

/// A logical mesh's morph definition, shared by the projections on all of its primitives.
#[derive(Clone, Copy)]
pub struct MorphTargetDefinitionRef<'s> {
    scene: &'s ValidatedScene,
    row: usize,
}

impl<'s> MorphTargetDefinitionRef<'s> {
    fn at(scene: &'s ValidatedScene, row: usize) -> Self {
        Self { scene, row }
    }

    /// Returns this logical target's generation- and mesh-qualified identity.
    pub fn id(self) -> MorphTargetId {
        let definition = self.read();
        MorphTargetId::new(
            LogicalMeshRef::at(self.scene, definition.logical_mesh()).id(),
            definition.target_ordinal() as u32,
        )
    }

    pub fn mesh(self) -> LogicalMeshRef<'s> {
        LogicalMeshRef::at(self.scene, self.read().logical_mesh())
    }

    /// Returns this target's position in the mesh default and instance override weight vectors.
    pub fn ordinal(self) -> u32 {
        self.read().target_ordinal() as u32
    }

    /// Returns the retained producer identity, independently of the current target ordinal.
    pub fn stable_identity(self) -> u32 {
        self.read().stable_identity()
    }

    pub fn name(self) -> Option<&'s str> {
        self.read().name()
    }

    fn read(self) -> NodeSceneDeformationMorphDefinition<'s> {
        expect_validated(
            tables(self.scene)
                .morph_definition_at(self.scene.as_node_scene().read_proof(), self.row),
        )
    }
}

impl fmt::Debug for MorphTargetDefinitionRef<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MorphTargetDefinitionRef")
            .field("id", &self.id())
            .finish_non_exhaustive()
    }
}

/// A primitive's projection of one logical morph target.
#[derive(Clone, Copy)]
pub struct MorphTargetRef<'s> {
    primitive: PrimitiveRef<'s>,
    row: usize,
}

impl<'s> MorphTargetRef<'s> {
    /// All primitive projections of the same logical target share this identity.
    pub fn id(self) -> MorphTargetId {
        MorphTargetId::new(
            self.primitive.mesh().id(),
            self.read().target_ordinal() as u32,
        )
    }

    pub const fn primitive(self) -> PrimitiveRef<'s> {
        self.primitive
    }

    pub fn definition(self) -> MorphTargetDefinitionRef<'s> {
        let mesh = self.primitive.mesh();
        let scene = mesh.scene();
        let logical = expect_validated(tables(scene).logical_morph_at(
            scene.as_node_scene().read_proof(),
            mesh.id().index() as usize,
        ));
        MorphTargetDefinitionRef::at(
            scene,
            logical.definition_offset() + self.read().target_ordinal(),
        )
    }

    pub fn delta_count(self) -> usize {
        self.read().delta_count()
    }

    /// Iterates the authored Position, Normal, Tangent streams in their retained order.
    pub fn deltas(
        self,
    ) -> impl ExactSizeIterator<Item = MorphDeltaRef<'s>> + DoubleEndedIterator + Clone + 's {
        let target = self.read();
        let scene = self.primitive.scene();
        let start = target.delta_offset();
        (start..start + target.delta_count()).map(move |row| {
            MorphDeltaRef::from_semantic(expect_validated(
                tables(scene).morph_delta_at(scene.as_node_scene().read_proof(), row),
            ))
        })
    }

    /// Looks up one semantic in this target's at most three retained delta streams.
    pub fn delta(self, semantic: MorphDeltaSemantic) -> Option<MorphDeltaRef<'s>> {
        self.deltas().find(|delta| delta.semantic() == semantic)
    }

    fn read(self) -> NodeSceneDeformationMorphTarget {
        let scene = self.primitive.scene();
        expect_validated(
            tables(scene).morph_target_at(scene.as_node_scene().read_proof(), self.row),
        )
    }
}

impl fmt::Debug for MorphTargetRef<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MorphTargetRef")
            .field("id", &self.id())
            .field("primitive", &self.primitive)
            .finish_non_exhaustive()
    }
}

/// Relative F32x3 deltas in the primitive's vertex domain.
///
/// `values` contains tightly packed XYZ triples. Sparse indices are strictly increasing;
/// unlisted vertices have the zero delta. No dense expansion is performed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MorphDeltaData<'s> {
    Dense {
        values: &'s [f32],
    },
    Sparse {
        logical_count: u32,
        indices: &'s [u32],
        values: &'s [f32],
    },
}

impl MorphDeltaData<'_> {
    /// Number of logical vertices, including implicit sparse zero deltas.
    pub const fn len(self) -> usize {
        match self {
            Self::Dense { values } => values.len() / 3,
            Self::Sparse { logical_count, .. } => logical_count as usize,
        }
    }

    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// Reads one delta in O(1) for dense data or O(log explicit count) for sparse data.
    pub fn value(self, vertex: usize) -> Option<Vec3> {
        let (values, index) = match self {
            Self::Dense { values } => (values, vertex),
            Self::Sparse {
                logical_count,
                indices,
                values,
            } => {
                if vertex >= logical_count as usize {
                    return None;
                }
                match indices.binary_search(&u32::try_from(vertex).ok()?) {
                    Ok(index) => (values, index),
                    Err(_) => return Some(Vec3::ZERO),
                }
            }
        };
        let start = index.checked_mul(3)?;
        let values = values.get(start..start.checked_add(3)?)?;
        Some(Vec3::new(values[0], values[1], values[2]))
    }
}

/// One borrowed relative delta stream with its authored semantic and storage representation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MorphDeltaRef<'s> {
    semantic: MorphDeltaSemantic,
    data: MorphDeltaData<'s>,
}

impl<'s> MorphDeltaRef<'s> {
    fn from_semantic(delta: NodeSceneDeformationMorphDelta<'s>) -> Self {
        let semantic = delta.semantic();
        let data = match delta.into_payload() {
            NodeSceneDeformationMorphPayload::Dense { values, .. } => MorphDeltaData::Dense {
                values: values.into_slice(),
            },
            NodeSceneDeformationMorphPayload::Sparse {
                logical_count,
                values,
            } => {
                let (indices, values) = values.into_slices();
                MorphDeltaData::Sparse {
                    logical_count: logical_count as u32,
                    indices,
                    values,
                }
            }
        };
        Self { semantic, data }
    }

    pub const fn semantic(self) -> MorphDeltaSemantic {
        self.semantic
    }

    pub const fn len(self) -> usize {
        self.data.len()
    }

    pub const fn is_empty(self) -> bool {
        self.data.is_empty()
    }

    pub const fn data(self) -> MorphDeltaData<'s> {
        self.data
    }
}

impl<'s> LogicalMeshRef<'s> {
    pub fn morph_target_count(self) -> usize {
        let scene = self.scene();
        expect_validated(tables(scene).logical_morph_at(
            scene.as_node_scene().read_proof(),
            self.id().index() as usize,
        ))
        .definition_count()
    }

    pub fn morph_target_definitions(
        self,
    ) -> impl ExactSizeIterator<Item = MorphTargetDefinitionRef<'s>> + DoubleEndedIterator + Clone + 's
    {
        let scene = self.scene();
        let logical = expect_validated(tables(scene).logical_morph_at(
            scene.as_node_scene().read_proof(),
            self.id().index() as usize,
        ));
        let start = logical.definition_offset();
        (start..start + logical.definition_count())
            .map(move |row| MorphTargetDefinitionRef::at(scene, row))
    }

    pub fn morph_target_definition(
        self,
        id: MorphTargetId,
    ) -> Result<MorphTargetDefinitionRef<'s>, SceneLookupError> {
        let scene = self.scene();
        let logical = expect_validated(tables(scene).logical_morph_at(
            scene.as_node_scene().read_proof(),
            self.id().index() as usize,
        ));
        let local = checked_morph_ordinal(self, id, logical.definition_count())?;
        Ok(MorphTargetDefinitionRef::at(
            scene,
            logical.definition_offset() + local,
        ))
    }

    /// Borrows defaults in logical target order, including an authored empty vector.
    pub fn default_morph_weights(self) -> &'s [f32] {
        let scene = self.scene();
        expect_validated(tables(scene).logical_morph_at(
            scene.as_node_scene().read_proof(),
            self.id().index() as usize,
        ))
        .take_default_weights()
        .expect("validated mesh has default morph weights")
        .into_slice()
    }
}

impl<'s> PrimitiveRef<'s> {
    /// Returns absent influence data distinctly from present rows with no influences.
    pub fn influences(self) -> Option<InfluenceRows<'s>> {
        let scene = self.scene();
        let (offsets, joints, weights, source_orders) = expect_validated(
            tables(scene).primitive_at(scene.as_node_scene().read_proof(), self.row()),
        )
        .take_influences()?
        .into_slices();
        Some(InfluenceRows {
            offsets,
            joints,
            weights,
            source_orders,
        })
    }

    pub fn morph_target_count(self) -> usize {
        let scene = self.scene();
        expect_validated(tables(scene).primitive_at(scene.as_node_scene().read_proof(), self.row()))
            .morph_target_count()
    }

    pub fn morph_targets(
        self,
    ) -> impl ExactSizeIterator<Item = MorphTargetRef<'s>> + DoubleEndedIterator + Clone + 's {
        let scene = self.scene();
        let primitive = expect_validated(
            tables(scene).primitive_at(scene.as_node_scene().read_proof(), self.row()),
        );
        let start = primitive.morph_target_offset();
        (start..start + primitive.morph_target_count()).map(move |row| MorphTargetRef {
            primitive: self,
            row,
        })
    }

    /// Finds this primitive's projection after checking the logical target's generation and owner.
    pub fn morph_target(self, id: MorphTargetId) -> Result<MorphTargetRef<'s>, SceneLookupError> {
        let scene = self.scene();
        let primitive = expect_validated(
            tables(scene).primitive_at(scene.as_node_scene().read_proof(), self.row()),
        );
        let local = checked_morph_ordinal(self.mesh(), id, primitive.morph_target_count())?;
        Ok(MorphTargetRef {
            primitive: self,
            row: primitive.morph_target_offset() + local,
        })
    }
}

impl<'s> MeshInstanceRef<'s> {
    /// Follows the instance's optional skin palette without another checked lookup.
    pub fn skin(self) -> Option<SkinRef<'s>> {
        self.read_deformation()
            .skin()
            .map(|index| SkinRef::at(self.scene(), index))
    }

    /// Borrows the complete authored override in logical target order.
    ///
    /// `None` means use the logical mesh defaults. `Some(&[])` preserves a present empty override.
    pub fn morph_weights(self) -> Option<&'s [f32]> {
        self.read_deformation()
            .take_morph_weights()
            .map(|values| values.into_slice())
    }

    fn read_deformation(self) -> NodeSceneDeformationMeshInstance<'s> {
        let scene = self.scene();
        expect_validated(tables(scene).mesh_instance_at(
            scene.as_node_scene().read_proof(),
            self.id().index() as usize,
        ))
    }
}

fn checked_morph_ordinal(
    mesh: LogicalMeshRef<'_>,
    id: MorphTargetId,
    count: usize,
) -> Result<usize, SceneLookupError> {
    let scene = mesh.scene();
    if id.generation() == scene.generation() && id.logical_mesh() != mesh.id() {
        return Err(SceneLookupError::OwnerMismatch {
            kind: SceneEntityKind::MorphTarget,
            expected: mesh.id(),
            actual: id.logical_mesh(),
        });
    }
    checked_index(
        scene,
        SceneEntityKind::MorphTarget,
        id.generation(),
        id.local(),
        count,
    )
}

impl ValidatedScene {
    pub fn skin_count(&self) -> usize {
        self.as_node_scene().read_proof().skin_count()
    }

    pub fn skins(
        &self,
    ) -> impl ExactSizeIterator<Item = SkinRef<'_>> + DoubleEndedIterator + Clone {
        (0..self.skin_count()).map(|index| SkinRef::at(self, index))
    }

    pub fn skin(&self, id: SkinId) -> Result<SkinRef<'_>, SceneLookupError> {
        let index = checked_index(
            self,
            SceneEntityKind::Skin,
            id.generation(),
            id.index(),
            self.skin_count(),
        )?;
        Ok(SkinRef::at(self, index))
    }

    /// Returns the number of logical target definitions, shared across primitive projections.
    pub fn morph_target_definition_count(&self) -> usize {
        self.as_node_scene().read_proof().morph_definition_count()
    }

    pub fn morph_target_definition(
        &self,
        id: MorphTargetId,
    ) -> Result<MorphTargetDefinitionRef<'_>, SceneLookupError> {
        checked_index(
            self,
            SceneEntityKind::MorphTarget,
            id.generation(),
            id.logical_mesh().index(),
            self.mesh_count(),
        )?;
        self.mesh(id.logical_mesh())?.morph_target_definition(id)
    }
}
