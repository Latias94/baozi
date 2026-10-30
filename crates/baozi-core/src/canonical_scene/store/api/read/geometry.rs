use std::fmt;

use super::super::super::construction::node_scene::semantic::geometry::{
    NodeSceneGeometryAttribute, NodeSceneGeometryAttributePayload,
    NodeSceneGeometryAttributeSemantic, NodeSceneGeometryLogicalMesh,
    NodeSceneGeometryMeshInstance, NodeSceneGeometryPrimitive, NodeSceneGeometryTables,
};
use super::{ValidatedScene, checked_index, expect_validated};
use crate::attribute::AttributeDomain;
use crate::geometry::TopologyKind;
use crate::identity::{
    LogicalMeshId, MeshInstanceId, PrimitiveId, SceneEntityKind, SceneLookupError,
};

fn tables(scene: &ValidatedScene) -> NodeSceneGeometryTables<'_> {
    expect_validated(NodeSceneGeometryTables::try_new(
        scene.as_node_scene().read_geometry(),
        scene.as_node_scene().read_proof(),
    ))
}

/// An allocation-free semantic, including borrowed namespaced custom names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeSemanticRef<'s> {
    Position,
    Normal,
    Tangent,
    Texcoord(u32),
    Color(u32),
    SmoothingGroup,
    Custom(&'s str),
}

/// Typed scalar slices for one attribute. Sparse indices are sorted logical element indices;
/// `default` is one element and `values` contains one element per explicit index.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AttributeData<'s> {
    DenseF32 {
        lanes: u8,
        values: &'s [f32],
    },
    DenseF64 {
        lanes: u8,
        values: &'s [f64],
    },
    DenseU32 {
        lanes: u8,
        values: &'s [u32],
    },
    DenseI32 {
        lanes: u8,
        values: &'s [i32],
    },
    SparseF32 {
        lanes: u8,
        logical_count: u32,
        indices: &'s [u32],
        values: &'s [f32],
        default: &'s [f32],
    },
    SparseU32 {
        lanes: u8,
        logical_count: u32,
        indices: &'s [u32],
        values: &'s [u32],
        default: &'s [u32],
    },
    SparseI32 {
        lanes: u8,
        logical_count: u32,
        indices: &'s [u32],
        values: &'s [i32],
        default: &'s [i32],
    },
}

/// One primitive attribute, borrowing its descriptor and payload from the Scene.
#[derive(Debug, Clone, Copy)]
pub struct AttributeRef<'s> {
    semantic: AttributeSemanticRef<'s>,
    domain: AttributeDomain,
    lanes: u8,
    count: u32,
    data: AttributeData<'s>,
}

impl<'s> AttributeRef<'s> {
    fn from_semantic(attribute: NodeSceneGeometryAttribute<'s>) -> Self {
        use NodeSceneGeometryAttributePayload as P;
        use NodeSceneGeometryAttributeSemantic as S;
        let semantic = match attribute.semantic() {
            S::Position => AttributeSemanticRef::Position,
            S::Normal => AttributeSemanticRef::Normal,
            S::Tangent => AttributeSemanticRef::Tangent,
            S::Texcoord(set) => AttributeSemanticRef::Texcoord(set),
            S::Color(set) => AttributeSemanticRef::Color(set),
            S::SmoothingGroup => AttributeSemanticRef::SmoothingGroup,
            S::Custom(name) => AttributeSemanticRef::Custom(name),
        };
        let domain = attribute.domain();
        let lanes = attribute.lanes();
        let count = attribute.element_count();
        let data = match attribute.into_payload() {
            P::DenseF32 { values } => AttributeData::DenseF32 {
                lanes,
                values: values.into_slice(),
            },
            P::DenseF64 { values } => AttributeData::DenseF64 {
                lanes,
                values: values.into_slice(),
            },
            P::DenseU32 { values } => AttributeData::DenseU32 {
                lanes,
                values: values.into_slice(),
            },
            P::DenseI32 { values } => AttributeData::DenseI32 {
                lanes,
                values: values.into_slice(),
            },
            P::SparseF32 {
                values,
                indices,
                default,
            } => AttributeData::SparseF32 {
                lanes,
                logical_count: count,
                indices: indices.into_slice(),
                values: values.into_slice(),
                default: default.into_slice(),
            },
            P::SparseU32 {
                values,
                indices,
                default,
            } => AttributeData::SparseU32 {
                lanes,
                logical_count: count,
                indices: indices.into_slice(),
                values: values.into_slice(),
                default: default.into_slice(),
            },
            P::SparseI32 {
                values,
                indices,
                default,
            } => AttributeData::SparseI32 {
                lanes,
                logical_count: count,
                indices: indices.into_slice(),
                values: values.into_slice(),
                default: default.into_slice(),
            },
        };
        Self {
            semantic,
            domain,
            lanes,
            count,
            data,
        }
    }

    pub const fn semantic(self) -> AttributeSemanticRef<'s> {
        self.semantic
    }
    pub const fn domain(self) -> AttributeDomain {
        self.domain
    }
    pub const fn lanes(self) -> u8 {
        self.lanes
    }
    /// Number of logical elements, including implicit sparse defaults.
    pub const fn len(self) -> usize {
        self.count as usize
    }
    pub const fn is_empty(self) -> bool {
        self.count == 0
    }
    pub const fn data(self) -> AttributeData<'s> {
        self.data
    }
}

/// A logical mesh with an ordered primitive range, independent of its placements.
#[derive(Clone, Copy)]
pub struct LogicalMeshRef<'s> {
    scene: &'s ValidatedScene,
    index: usize,
}

impl<'s> LogicalMeshRef<'s> {
    pub(super) const fn scene(self) -> &'s ValidatedScene {
        self.scene
    }

    pub(super) const fn at(scene: &'s ValidatedScene, index: usize) -> Self {
        Self { scene, index }
    }

    fn read(self) -> NodeSceneGeometryLogicalMesh<'s> {
        expect_validated(
            tables(self.scene).logical_mesh_at(self.scene.as_node_scene().read_proof(), self.index),
        )
    }
    pub fn id(self) -> LogicalMeshId {
        LogicalMeshId::new(self.scene.generation(), self.index as u32)
    }
    pub fn name(self) -> Option<&'s str> {
        self.read().name()
    }
    pub fn primitive_count(self) -> usize {
        self.read().primitive_count()
    }
    pub fn primitives(
        self,
    ) -> impl ExactSizeIterator<Item = PrimitiveRef<'s>> + DoubleEndedIterator + Clone + 's {
        let mesh = self.read();
        (0..mesh.primitive_count()).map(move |local| PrimitiveRef {
            scene: self.scene,
            mesh: self.index,
            row: mesh.primitive_offset() + local,
            local,
        })
    }
    pub fn primitive(self, id: PrimitiveId) -> Result<PrimitiveRef<'s>, SceneLookupError> {
        let mesh = self.read();
        if id.generation() == self.scene.generation() && id.logical_mesh() != self.id() {
            return Err(SceneLookupError::OwnerMismatch {
                kind: SceneEntityKind::Primitive,
                expected: self.id(),
                actual: id.logical_mesh(),
            });
        }
        let local = checked_index(
            self.scene,
            SceneEntityKind::Primitive,
            id.generation(),
            id.local(),
            mesh.primitive_count(),
        )?;
        Ok(PrimitiveRef {
            scene: self.scene,
            mesh: self.index,
            row: mesh.primitive_offset() + local,
            local,
        })
    }
}

/// One primitive; topology and domain-specific attributes retain their authored meaning.
///
/// Payload borrows cannot outlive their owning Scene:
///
/// ```compile_fail,E0515
/// use baozi_core::{AttributeData, ValidatedScene};
///
/// fn escape_positions(scene: ValidatedScene) -> &'static [f32] {
///     let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
///     let AttributeData::DenseF32 { values, .. } = primitive.positions().unwrap().data() else {
///         panic!("expected F32 positions")
///     };
///     values
/// }
/// ```
#[derive(Clone, Copy)]
pub struct PrimitiveRef<'s> {
    scene: &'s ValidatedScene,
    mesh: usize,
    row: usize,
    local: usize,
}

impl<'s> PrimitiveRef<'s> {
    pub(super) const fn scene(self) -> &'s ValidatedScene {
        self.scene
    }

    pub(super) const fn row(self) -> usize {
        self.row
    }

    fn read(self) -> NodeSceneGeometryPrimitive<'s> {
        expect_validated(
            tables(self.scene).primitive_at(self.scene.as_node_scene().read_proof(), self.row),
        )
    }
    pub fn id(self) -> PrimitiveId {
        PrimitiveId::new(
            LogicalMeshId::new(self.scene.generation(), self.mesh as u32),
            self.local as u32,
        )
    }
    pub fn mesh(self) -> LogicalMeshRef<'s> {
        LogicalMeshRef::at(self.scene, self.mesh)
    }
    pub fn topology(self) -> TopologyKind {
        self.read().topology()
    }
    pub fn vertex_count(self) -> u32 {
        self.read().vertex_count()
    }
    pub fn corner_count(self) -> u32 {
        self.read().corner_count()
    }
    pub fn face_count(self) -> u32 {
        self.read().face_count()
    }
    /// Absent indices denote the validated implicit index stream; authored emptiness is preserved.
    pub fn indices(self) -> Option<&'s [u32]> {
        self.read().take_indices().map(|values| values.into_slice())
    }
    /// Number of corners in each polygon face, present only for polygon topology.
    pub fn polygon_face_sizes(self) -> Option<&'s [u32]> {
        self.read()
            .take_polygon_boundaries()
            .map(|values| values.into_slice())
    }
    pub fn material(self) -> Option<super::MaterialRef<'s>> {
        self.read()
            .material()
            .map(|index| super::MaterialRef::at(self.scene, index))
    }
    pub fn attributes(
        self,
    ) -> impl ExactSizeIterator<Item = AttributeRef<'s>> + DoubleEndedIterator + Clone + 's {
        let primitive = self.read();
        let offset = primitive.attribute_offset();
        (offset..offset + primitive.attribute_count()).map(move |row| {
            AttributeRef::from_semantic(expect_validated(
                tables(self.scene).attribute_at(self.scene.as_node_scene().read_proof(), row),
            ))
        })
    }
    /// Looks up an exact semantic/domain pair in O(attribute count), without copying its values.
    pub fn attribute(
        self,
        semantic: AttributeSemanticRef<'_>,
        domain: AttributeDomain,
    ) -> Option<AttributeRef<'s>> {
        self.attributes()
            .find(|attribute| attribute.semantic() == semantic && attribute.domain() == domain)
    }
    /// Returns the Vertex-domain position attribute.
    pub fn positions(self) -> Option<AttributeRef<'s>> {
        self.attribute(AttributeSemanticRef::Position, AttributeDomain::Vertex)
    }
    /// Returns the normal attribute in the requested domain.
    pub fn normals(self, domain: AttributeDomain) -> Option<AttributeRef<'s>> {
        self.attribute(AttributeSemanticRef::Normal, domain)
    }
    /// Returns the texture-coordinate set in the requested domain.
    pub fn texcoords(self, set: u32, domain: AttributeDomain) -> Option<AttributeRef<'s>> {
        self.attribute(AttributeSemanticRef::Texcoord(set), domain)
    }
}

/// One mesh placement. Following its node or mesh does not allocate or repeat lookup checks.
#[derive(Clone, Copy)]
pub struct MeshInstanceRef<'s> {
    scene: &'s ValidatedScene,
    index: usize,
}

impl<'s> MeshInstanceRef<'s> {
    pub(super) const fn scene(self) -> &'s ValidatedScene {
        self.scene
    }

    fn read(self) -> NodeSceneGeometryMeshInstance {
        expect_validated(
            tables(self.scene)
                .mesh_instance_at(self.scene.as_node_scene().read_proof(), self.index),
        )
    }
    pub fn id(self) -> MeshInstanceId {
        MeshInstanceId::new(self.scene.generation(), self.index as u32)
    }
    pub fn node(self) -> super::NodeRef<'s> {
        super::NodeRef::at(self.scene, self.read().owner_node())
    }
    pub fn mesh(self) -> LogicalMeshRef<'s> {
        LogicalMeshRef::at(self.scene, self.read().logical_mesh())
    }
}

impl fmt::Debug for LogicalMeshRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LogicalMeshRef")
            .field("id", &self.id())
            .finish_non_exhaustive()
    }
}
impl fmt::Debug for PrimitiveRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PrimitiveRef")
            .field("id", &self.id())
            .finish_non_exhaustive()
    }
}
impl fmt::Debug for MeshInstanceRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MeshInstanceRef")
            .field("id", &self.id())
            .finish_non_exhaustive()
    }
}

impl ValidatedScene {
    pub fn mesh_count(&self) -> usize {
        self.as_node_scene().read_proof().logical_mesh_count()
    }
    pub fn primitive_count(&self) -> usize {
        self.as_node_scene().read_proof().primitive_count()
    }
    pub fn attribute_count(&self) -> usize {
        self.as_node_scene().read_proof().attribute_count()
    }
    pub fn mesh_instance_count(&self) -> usize {
        self.as_node_scene().read_proof().mesh_instance_count()
    }
    pub fn meshes(
        &self,
    ) -> impl ExactSizeIterator<Item = LogicalMeshRef<'_>> + DoubleEndedIterator + Clone {
        (0..self.mesh_count()).map(|index| LogicalMeshRef::at(self, index))
    }
    pub fn mesh_instances(
        &self,
    ) -> impl ExactSizeIterator<Item = MeshInstanceRef<'_>> + DoubleEndedIterator + Clone {
        (0..self.mesh_instance_count()).map(|index| MeshInstanceRef { scene: self, index })
    }
    pub fn mesh(&self, id: LogicalMeshId) -> Result<LogicalMeshRef<'_>, SceneLookupError> {
        let index = checked_index(
            self,
            SceneEntityKind::LogicalMesh,
            id.generation(),
            id.index(),
            self.mesh_count(),
        )?;
        Ok(LogicalMeshRef::at(self, index))
    }
    pub fn primitive(&self, id: PrimitiveId) -> Result<PrimitiveRef<'_>, SceneLookupError> {
        checked_index(
            self,
            SceneEntityKind::Primitive,
            id.generation(),
            id.logical_mesh().index(),
            self.mesh_count(),
        )?;
        self.mesh(id.logical_mesh())?.primitive(id)
    }
    pub fn mesh_instance(
        &self,
        id: MeshInstanceId,
    ) -> Result<MeshInstanceRef<'_>, SceneLookupError> {
        let index = checked_index(
            self,
            SceneEntityKind::MeshInstance,
            id.generation(),
            id.index(),
            self.mesh_instance_count(),
        )?;
        Ok(MeshInstanceRef { scene: self, index })
    }
}
