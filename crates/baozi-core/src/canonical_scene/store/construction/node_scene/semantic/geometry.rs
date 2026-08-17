use baozi_operation::{
    CanonicalRecordView, PublishedCanonicalStore, RecordBlock, SequenceView, StoredScalar,
    Utf8Block,
};

use crate::attribute::AttributeDomain;
use crate::canonical_scene::store::schema::appearance::AppearanceMaterialTableSchema;
use crate::canonical_scene::store::schema::geometry::{
    GEOMETRY_ATTRIBUTE_FIELD_COUNT, GEOMETRY_DOMAIN_CORNER, GEOMETRY_DOMAIN_FACE,
    GEOMETRY_DOMAIN_VERTEX, GEOMETRY_LOGICAL_MESH_FIELD_COUNT, GEOMETRY_MESH_INSTANCE_FIELD_COUNT,
    GEOMETRY_NORMALIZATION_NONE, GEOMETRY_PAYLOAD_DENSE, GEOMETRY_PAYLOAD_SPARSE,
    GEOMETRY_PRIMITIVE_FIELD_COUNT, GEOMETRY_SCALAR_F32, GEOMETRY_SCALAR_I32, GEOMETRY_SCALAR_U32,
    GEOMETRY_SEMANTIC_COLOR, GEOMETRY_SEMANTIC_CUSTOM, GEOMETRY_SEMANTIC_NORMAL,
    GEOMETRY_SEMANTIC_POSITION, GEOMETRY_SEMANTIC_SMOOTHING_GROUP, GEOMETRY_SEMANTIC_TANGENT,
    GEOMETRY_SEMANTIC_TEXCOORD, GEOMETRY_TOPOLOGY_LINE_LIST, GEOMETRY_TOPOLOGY_LINE_LOOP,
    GEOMETRY_TOPOLOGY_LINE_STRIP, GEOMETRY_TOPOLOGY_POINTS, GEOMETRY_TOPOLOGY_POLYGONS,
    GEOMETRY_TOPOLOGY_TRIANGLE_FAN, GEOMETRY_TOPOLOGY_TRIANGLE_LIST,
    GEOMETRY_TOPOLOGY_TRIANGLE_STRIP, GeometryAttributeSemanticNameSchema, GeometryF32ValuesSchema,
    GeometryI32ValuesSchema, GeometryLogicalMeshNameSchema, GeometryPolygonBoundariesSchema,
    GeometryPrimitiveIndicesSchema, GeometrySparseF32DefaultsSchema,
    GeometrySparseI32DefaultsSchema, GeometrySparseIndicesSchema, GeometrySparseU32DefaultsSchema,
    GeometryU32ValuesSchema, geometry_attribute_field, geometry_logical_mesh_field,
    geometry_mesh_instance_field, geometry_primitive_field,
};
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, CanonicalSchema, PRESENCE_ABSENT, PRESENCE_PRESENT, SceneDirectorySchema,
};
use crate::canonical_scene::store::validation::NodeSceneValidationProof;
use crate::geometry::TopologyKind;

use super::{NodeSceneSemanticResult, invariant, read_scalar, read_u32_as_usize};

#[cfg(feature = "_u7-geometry")]
mod inspection;

#[cfg(feature = "_u7-geometry")]
pub(in crate::canonical_scene::store) use inspection::{
    NodeSceneGeometryInspectionCursor, NodeSceneGeometryPrimitiveFacts,
};

type SceneCanonicalStore = PublishedCanonicalStore<RecordBlock, SceneDirectorySchema>;

/// The four proof-bound geometry table projections retained by the one Scene.
pub(super) struct NodeSceneGeometryTables<'scene> {
    canonical: &'scene SceneCanonicalStore,
    logical_meshes: Option<CanonicalRecordView<'scene>>,
    primitives: Option<CanonicalRecordView<'scene>>,
    attributes: Option<CanonicalRecordView<'scene>>,
    mesh_instances: Option<CanonicalRecordView<'scene>>,
    logical_mesh_names: Option<&'scene str>,
    attribute_semantic_names: Option<&'scene str>,
}

impl<'scene> NodeSceneGeometryTables<'scene> {
    pub(super) fn try_new(
        canonical: &'scene SceneCanonicalStore,
        proof: NodeSceneValidationProof,
    ) -> NodeSceneSemanticResult<Self> {
        let logical_meshes = resolve_table(
            canonical,
            proof.logical_meshes(),
            proof.logical_mesh_count(),
            GEOMETRY_LOGICAL_MESH_FIELD_COUNT,
            "validated logical-mesh table is unavailable or mistyped",
            "validated logical-mesh table presence disagrees with validation proof",
            "validated logical-mesh table layout disagrees with validation proof",
        )?;
        let primitives = resolve_table(
            canonical,
            proof.primitives(),
            proof.primitive_count(),
            GEOMETRY_PRIMITIVE_FIELD_COUNT,
            "validated primitive table is unavailable or mistyped",
            "validated primitive table presence disagrees with validation proof",
            "validated primitive table layout disagrees with validation proof",
        )?;
        let attributes = resolve_table(
            canonical,
            proof.attributes(),
            proof.attribute_count(),
            GEOMETRY_ATTRIBUTE_FIELD_COUNT,
            "validated attribute table is unavailable or mistyped",
            "validated attribute table presence disagrees with validation proof",
            "validated attribute table layout disagrees with validation proof",
        )?;
        let mesh_instances = resolve_table(
            canonical,
            proof.mesh_instances(),
            proof.mesh_instance_count(),
            GEOMETRY_MESH_INSTANCE_FIELD_COUNT,
            "validated mesh-instance table is unavailable or mistyped",
            "validated mesh-instance table presence disagrees with validation proof",
            "validated mesh-instance table layout disagrees with validation proof",
        )?;
        let logical_mesh_names = resolve_utf8(
            canonical,
            proof.logical_mesh_names(),
            "validated logical-mesh-name payload is unavailable or mistyped",
        )?;
        let attribute_semantic_names = resolve_utf8(
            canonical,
            proof.attribute_semantic_names(),
            "validated attribute-semantic-name payload is unavailable or mistyped",
        )?;

        Ok(Self {
            canonical,
            logical_meshes,
            primitives,
            attributes,
            mesh_instances,
            logical_mesh_names,
            attribute_semantic_names,
        })
    }

    pub(super) const fn into_cursor(
        self,
        proof: NodeSceneValidationProof,
    ) -> NodeSceneGeometryCursor<'scene> {
        NodeSceneGeometryCursor {
            tables: self,
            proof,
            phase: GeometryCursorPhase::LogicalMeshes,
            next_logical_mesh: 0,
            next_logical_mesh_name_offset: 0,
            next_primitive: 0,
            next_attribute: 0,
            next_attribute_semantic_name_offset: 0,
            next_mesh_instance: 0,
        }
    }
}

fn resolve_utf8<'scene, Tag>(
    canonical: &'scene SceneCanonicalStore,
    reference: Option<CanonicalBlockRef<Tag>>,
    unavailable_reason: &'static str,
) -> NodeSceneSemanticResult<Option<&'scene str>>
where
    Tag: CanonicalSchema<StoreKind = Utf8Block>,
{
    reference
        .map(|reference| {
            canonical
                .utf8(reference.into_store())
                .ok_or_else(|| invariant(unavailable_reason))
        })
        .transpose()
}

fn resolve_table<'scene, Tag>(
    canonical: &'scene SceneCanonicalStore,
    reference: Option<CanonicalBlockRef<Tag>>,
    expected_rows: usize,
    expected_fields: usize,
    unavailable_reason: &'static str,
    presence_reason: &'static str,
    layout_reason: &'static str,
) -> NodeSceneSemanticResult<Option<CanonicalRecordView<'scene>>>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    match (expected_rows, reference) {
        (0, None) => Ok(None),
        (0, Some(_)) | (_, None) => Err(invariant(presence_reason)),
        (_, Some(reference)) => {
            let table = canonical
                .records(reference.into_store())
                .ok_or_else(|| invariant(unavailable_reason))?;
            if table.rows() != expected_rows || table.fields() != expected_fields {
                return Err(invariant(layout_reason));
            }
            Ok(Some(table))
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum GeometryCursorPhase {
    LogicalMeshes,
    Primitives,
    Attributes,
    MeshInstances,
}

/// Linear allocation-free traversal of every proof-qualified geometry table.
#[must_use = "the geometry cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneGeometryCursor<'scene> {
    tables: NodeSceneGeometryTables<'scene>,
    proof: NodeSceneValidationProof,
    phase: GeometryCursorPhase,
    next_logical_mesh: usize,
    next_logical_mesh_name_offset: usize,
    next_primitive: usize,
    next_attribute: usize,
    next_attribute_semantic_name_offset: usize,
    next_mesh_instance: usize,
}

impl<'scene> NodeSceneGeometryCursor<'scene> {
    pub(in crate::canonical_scene::store) fn logical_mesh_count(&self) -> usize {
        self.proof.logical_mesh_count()
    }

    pub(in crate::canonical_scene::store) fn primitive_count(&self) -> usize {
        self.proof.primitive_count()
    }

    pub(in crate::canonical_scene::store) fn attribute_count(&self) -> usize {
        self.proof.attribute_count()
    }

    pub(in crate::canonical_scene::store) fn mesh_instance_count(&self) -> usize {
        self.proof.mesh_instance_count()
    }

    pub(in crate::canonical_scene::store) fn next_logical_mesh(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneGeometryLogicalMesh<'scene>>> {
        self.enter_phase(GeometryCursorPhase::LogicalMeshes)?;
        let Some(row) = next_row(
            self.next_logical_mesh,
            self.proof.logical_mesh_count(),
            "logical-mesh cursor advanced beyond its proof-qualified count",
        )?
        else {
            return Ok(None);
        };
        let table = self
            .tables
            .logical_meshes
            .ok_or_else(|| invariant("validated logical-mesh table view is absent"))?;
        require_row_ordinal(
            table,
            row,
            geometry_logical_mesh_field::LOGICAL_MESH,
            "validated logical-mesh identity is unreadable",
            "validated logical-mesh identity disagrees with its row",
        )?;
        let primitive_offset = read_u32_as_usize(
            table,
            row,
            geometry_logical_mesh_field::PRIMITIVE_OFFSET,
            "validated logical-mesh primitive offset is unreadable",
        )?;
        let primitive_count = read_u32_as_usize(
            table,
            row,
            geometry_logical_mesh_field::PRIMITIVE_COUNT,
            "validated logical-mesh primitive count is unreadable",
        )?;
        require_range(
            primitive_offset,
            primitive_count,
            self.proof.primitive_count(),
            "validated logical-mesh primitive range is inconsistent",
        )?;
        let primitives = self
            .proof
            .primitives()
            .ok_or_else(|| invariant("validated logical-mesh primitive-table proof is absent"))?;
        require_exact_record_reference(
            table,
            row,
            geometry_logical_mesh_field::PRIMITIVES,
            primitives,
            "validated logical-mesh primitive-table reference is unreadable",
        )?;
        let name_presence = read_scalar::<u8>(
            table,
            row,
            geometry_logical_mesh_field::NAME_PRESENCE,
            "validated logical-mesh name presence is unreadable",
        )?;
        let name_offset = read_u32_as_usize(
            table,
            row,
            geometry_logical_mesh_field::NAME_OFFSET,
            "validated logical-mesh name offset is unreadable",
        )?;
        let name_count = read_u32_as_usize(
            table,
            row,
            geometry_logical_mesh_field::NAME_COUNT,
            "validated logical-mesh name count is unreadable",
        )?;
        if name_offset != self.next_logical_mesh_name_offset {
            return Err(invariant(
                "validated logical-mesh-name ranges are not contiguous",
            ));
        }
        let (name, name_end) = match name_presence {
            PRESENCE_ABSENT if name_count == 0 => {
                if table.scalar::<u8>(row, geometry_logical_mesh_field::NAMES) != Some(0) {
                    return Err(invariant(
                        "validated absent logical-mesh name has a nonzero sentinel",
                    ));
                }
                (None, name_offset)
            }
            PRESENCE_PRESENT if name_count == 0 => {
                if table.scalar::<u8>(row, geometry_logical_mesh_field::NAMES) != Some(0) {
                    return Err(invariant(
                        "validated empty logical-mesh name has a nonzero sentinel",
                    ));
                }
                (Some(""), name_offset)
            }
            PRESENCE_PRESENT => {
                require_utf8_ref::<GeometryLogicalMeshNameSchema>(
                    table,
                    row,
                    geometry_logical_mesh_field::NAMES,
                    self.proof.logical_mesh_names(),
                    "validated logical-mesh-name reference is unreadable",
                    "validated logical-mesh-name reference disagrees with validation proof",
                )?;
                let end = name_offset
                    .checked_add(name_count)
                    .ok_or_else(|| invariant("validated logical-mesh name range overflows"))?;
                let value = self
                    .tables
                    .logical_mesh_names
                    .and_then(|names| names.get(name_offset..end))
                    .ok_or_else(|| invariant("validated logical-mesh name range is unreadable"))?;
                (Some(value), end)
            }
            _ => return Err(invariant("validated logical-mesh name presence is invalid")),
        };

        self.next_logical_mesh_name_offset = name_end;
        self.next_logical_mesh = advance_row(row, "logical-mesh cursor index overflows")?;
        Ok(Some(NodeSceneGeometryLogicalMesh {
            source_index: row,
            name,
            primitive_offset,
            primitive_count,
        }))
    }

    pub(in crate::canonical_scene::store) fn next_primitive(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneGeometryPrimitive<'scene>>> {
        self.enter_phase(GeometryCursorPhase::Primitives)?;
        let Some(row) = next_row(
            self.next_primitive,
            self.proof.primitive_count(),
            "primitive cursor advanced beyond its proof-qualified count",
        )?
        else {
            return Ok(None);
        };
        let table = self
            .tables
            .primitives
            .ok_or_else(|| invariant("validated primitive table view is absent"))?;
        require_row_ordinal(
            table,
            row,
            geometry_primitive_field::PRIMITIVE,
            "validated primitive identity is unreadable",
            "validated primitive identity disagrees with its row",
        )?;
        let topology = decode_topology(read_scalar::<u8>(
            table,
            row,
            geometry_primitive_field::TOPOLOGY,
            "validated primitive topology is unreadable",
        )?)?;
        let vertex_count = read_scalar::<u32>(
            table,
            row,
            geometry_primitive_field::VERTEX_COUNT,
            "validated primitive vertex count is unreadable",
        )?;
        let corner_count = read_scalar::<u32>(
            table,
            row,
            geometry_primitive_field::CORNER_COUNT,
            "validated primitive corner count is unreadable",
        )?;
        let face_count = read_scalar::<u32>(
            table,
            row,
            geometry_primitive_field::FACE_COUNT,
            "validated primitive face count is unreadable",
        )?;
        let polygon_boundaries = read_optional_u32_leaf::<GeometryPolygonBoundariesSchema>(
            self.tables.canonical,
            table,
            row,
            geometry_primitive_field::POLYGON_BOUNDARIES_PRESENCE,
            geometry_primitive_field::POLYGON_BOUNDARIES_COUNT,
            geometry_primitive_field::POLYGON_BOUNDARIES,
            "validated polygon-boundary presence is unreadable",
            "validated polygon-boundary count is unreadable",
            "validated polygon-boundary payload is unavailable or mistyped",
        )?;
        if matches!(topology, TopologyKind::Polygons) != polygon_boundaries.is_some() {
            return Err(invariant(
                "validated polygon-boundary presence disagrees with primitive topology",
            ));
        }
        let indices = read_optional_u32_leaf::<GeometryPrimitiveIndicesSchema>(
            self.tables.canonical,
            table,
            row,
            geometry_primitive_field::INDEX_PRESENCE,
            geometry_primitive_field::INDEX_COUNT,
            geometry_primitive_field::INDICES,
            "validated primitive-index presence is unreadable",
            "validated primitive-index count is unreadable",
            "validated primitive-index payload is unavailable or mistyped",
        )?;
        let material = read_optional_material(
            table,
            row,
            self.proof.material_count(),
            self.proof.materials(),
        )?;
        let attribute_offset = read_u32_as_usize(
            table,
            row,
            geometry_primitive_field::ATTRIBUTE_OFFSET,
            "validated primitive attribute offset is unreadable",
        )?;
        let attribute_count = read_u32_as_usize(
            table,
            row,
            geometry_primitive_field::ATTRIBUTE_COUNT,
            "validated primitive attribute count is unreadable",
        )?;
        require_range(
            attribute_offset,
            attribute_count,
            self.proof.attribute_count(),
            "validated primitive attribute range is inconsistent",
        )?;
        require_record_reference(
            table,
            row,
            geometry_primitive_field::ATTRIBUTES,
            self.proof.attributes(),
            attribute_count,
            "validated primitive attribute-table reference is unreadable",
        )?;

        let polygon_boundaries_present = polygon_boundaries.is_some();
        let polygon_boundary_count = polygon_boundaries
            .as_ref()
            .map_or(0, |values| values.declared_count());
        let indices_present = indices.is_some();
        let index_count = indices.as_ref().map_or(0, |values| values.declared_count());
        self.next_primitive = advance_row(row, "primitive cursor index overflows")?;
        Ok(Some(NodeSceneGeometryPrimitive {
            source_index: row,
            topology,
            vertex_count,
            corner_count,
            face_count,
            polygon_boundaries_present,
            polygon_boundary_count,
            polygon_boundaries,
            indices_present,
            index_count,
            indices,
            material,
            attribute_offset,
            attribute_count,
        }))
    }

    pub(in crate::canonical_scene::store) fn next_attribute(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneGeometryAttribute<'scene>>> {
        self.enter_phase(GeometryCursorPhase::Attributes)?;
        let Some(row) = next_row(
            self.next_attribute,
            self.proof.attribute_count(),
            "attribute cursor advanced beyond its proof-qualified count",
        )?
        else {
            return Ok(None);
        };
        let table = self
            .tables
            .attributes
            .ok_or_else(|| invariant("validated attribute table view is absent"))?;
        require_row_ordinal(
            table,
            row,
            geometry_attribute_field::ATTRIBUTE,
            "validated attribute identity is unreadable",
            "validated attribute identity disagrees with its row",
        )?;
        let semantic_kind = read_scalar::<u8>(
            table,
            row,
            geometry_attribute_field::SEMANTIC_KIND,
            "validated attribute semantic kind is unreadable",
        )?;
        let semantic_set = read_scalar::<u32>(
            table,
            row,
            geometry_attribute_field::SEMANTIC_SET,
            "validated attribute semantic set is unreadable",
        )?;
        let semantic_name_offset = read_u32_as_usize(
            table,
            row,
            geometry_attribute_field::SEMANTIC_NAME_OFFSET,
            "validated attribute semantic-name offset is unreadable",
        )?;
        let semantic_name_count = read_u32_as_usize(
            table,
            row,
            geometry_attribute_field::SEMANTIC_NAME_COUNT,
            "validated attribute semantic-name count is unreadable",
        )?;
        if semantic_name_offset != self.next_attribute_semantic_name_offset {
            return Err(invariant(
                "validated attribute-semantic-name ranges are not contiguous",
            ));
        }
        let (semantic_name, semantic_name_end) = if semantic_kind == GEOMETRY_SEMANTIC_CUSTOM {
            if semantic_name_count == 0 {
                return Err(invariant(
                    "validated custom attribute semantic has an empty name",
                ));
            }
            require_utf8_ref::<GeometryAttributeSemanticNameSchema>(
                table,
                row,
                geometry_attribute_field::SEMANTIC_NAMES,
                self.proof.attribute_semantic_names(),
                "validated attribute-semantic-name reference is unreadable",
                "validated attribute-semantic-name reference disagrees with validation proof",
            )?;
            let end = semantic_name_offset
                .checked_add(semantic_name_count)
                .ok_or_else(|| invariant("validated attribute semantic-name range overflows"))?;
            let name = self
                .tables
                .attribute_semantic_names
                .and_then(|names| names.get(semantic_name_offset..end))
                .ok_or_else(|| {
                    invariant("validated attribute semantic-name range is unreadable")
                })?;
            (Some(name), end)
        } else {
            if semantic_name_count != 0
                || table.scalar::<u8>(row, geometry_attribute_field::SEMANTIC_NAMES) != Some(0)
            {
                return Err(invariant(
                    "validated built-in attribute semantic retains a custom name",
                ));
            }
            (None, semantic_name_offset)
        };
        let semantic = decode_semantic(semantic_kind, semantic_set, semantic_name)?;
        let domain = decode_domain(read_scalar::<u8>(
            table,
            row,
            geometry_attribute_field::DOMAIN,
            "validated attribute domain is unreadable",
        )?)?;
        let scalar_kind = decode_scalar_kind(read_scalar::<u8>(
            table,
            row,
            geometry_attribute_field::SCALAR_KIND,
            "validated attribute scalar kind is unreadable",
        )?)?;
        let lanes = read_scalar::<u8>(
            table,
            row,
            geometry_attribute_field::LANE_COUNT,
            "validated attribute lane count is unreadable",
        )?;
        if lanes == 0 {
            return Err(invariant("validated attribute lane count is zero"));
        }
        let normalization = decode_normalization(read_scalar::<u8>(
            table,
            row,
            geometry_attribute_field::NORMALIZATION,
            "validated attribute normalization is unreadable",
        )?)?;
        let payload_kind = decode_payload_kind(read_scalar::<u8>(
            table,
            row,
            geometry_attribute_field::PAYLOAD_KIND,
            "validated attribute payload kind is unreadable",
        )?)?;
        let element_count = read_scalar::<u32>(
            table,
            row,
            geometry_attribute_field::ELEMENT_COUNT,
            "validated attribute element count is unreadable",
        )?;
        let explicit_element_count = read_scalar::<u32>(
            table,
            row,
            geometry_attribute_field::EXPLICIT_COUNT,
            "validated attribute explicit-element count is unreadable",
        )?;
        let canonical_scalar_count = read_scalar::<u64>(
            table,
            row,
            geometry_attribute_field::CANONICAL_SCALAR_COUNT,
            "validated attribute canonical-scalar count is unreadable",
        )?;
        let payload = read_attribute_payload(
            self.tables.canonical,
            table,
            row,
            scalar_kind,
            payload_kind,
            lanes,
            element_count,
            explicit_element_count,
            canonical_scalar_count,
        )?;

        self.next_attribute_semantic_name_offset = semantic_name_end;
        self.next_attribute = advance_row(row, "attribute cursor index overflows")?;
        Ok(Some(NodeSceneGeometryAttribute {
            source_index: row,
            semantic,
            domain,
            scalar_kind,
            lanes,
            normalization,
            payload_kind,
            element_count,
            explicit_element_count,
            canonical_scalar_count,
            payload,
        }))
    }

    pub(in crate::canonical_scene::store) fn next_mesh_instance(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneGeometryMeshInstance>> {
        self.enter_phase(GeometryCursorPhase::MeshInstances)?;
        let Some(row) = next_row(
            self.next_mesh_instance,
            self.proof.mesh_instance_count(),
            "mesh-instance cursor advanced beyond its proof-qualified count",
        )?
        else {
            return Ok(None);
        };
        let table = self
            .tables
            .mesh_instances
            .ok_or_else(|| invariant("validated mesh-instance table view is absent"))?;
        require_row_ordinal(
            table,
            row,
            geometry_mesh_instance_field::MESH_INSTANCE,
            "validated mesh-instance identity is unreadable",
            "validated mesh-instance identity disagrees with its row",
        )?;
        let owner_node = read_u32_as_usize(
            table,
            row,
            geometry_mesh_instance_field::OWNER_NODE,
            "validated mesh-instance owner node is unreadable",
        )?
        .checked_sub(1)
        .ok_or_else(|| {
            invariant("validated mesh-instance owner cannot address source-node space")
        })?;
        if owner_node >= self.proof.source_node_count() {
            return Err(invariant(
                "validated mesh-instance owner node is outside source-node space",
            ));
        }
        require_exact_record_reference(
            table,
            row,
            geometry_mesh_instance_field::NODES,
            self.proof.nodes(),
            "validated mesh-instance node-table reference is unreadable",
        )?;
        let logical_mesh = read_u32_as_usize(
            table,
            row,
            geometry_mesh_instance_field::LOGICAL_MESH,
            "validated mesh-instance logical mesh is unreadable",
        )?;
        if logical_mesh >= self.proof.logical_mesh_count() {
            return Err(invariant(
                "validated mesh-instance logical mesh is outside logical-mesh space",
            ));
        }
        let logical_meshes = self.proof.logical_meshes().ok_or_else(|| {
            invariant("validated mesh-instance logical-mesh table proof is absent")
        })?;
        require_exact_record_reference(
            table,
            row,
            geometry_mesh_instance_field::LOGICAL_MESHES,
            logical_meshes,
            "validated mesh-instance logical-mesh table reference is unreadable",
        )?;

        self.next_mesh_instance = advance_row(row, "mesh-instance cursor index overflows")?;
        Ok(Some(NodeSceneGeometryMeshInstance {
            source_index: row,
            owner_node,
            logical_mesh,
        }))
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        if self.next_logical_mesh != self.proof.logical_mesh_count() {
            return Err(invariant(
                "logical-mesh cursor ended before its proof-qualified count",
            ));
        }
        if self.next_logical_mesh_name_offset != self.tables.logical_mesh_names.map_or(0, str::len)
        {
            return Err(invariant(
                "logical-mesh-name cursor ended before its proof-qualified payload",
            ));
        }
        if self.next_primitive != self.proof.primitive_count() {
            return Err(invariant(
                "primitive cursor ended before its proof-qualified count",
            ));
        }
        if self.next_attribute != self.proof.attribute_count() {
            return Err(invariant(
                "attribute cursor ended before its proof-qualified count",
            ));
        }
        if self.next_attribute_semantic_name_offset
            != self.tables.attribute_semantic_names.map_or(0, str::len)
        {
            return Err(invariant(
                "attribute-semantic-name cursor ended before its proof-qualified payload",
            ));
        }
        if self.next_mesh_instance != self.proof.mesh_instance_count() {
            return Err(invariant(
                "mesh-instance cursor ended before its proof-qualified count",
            ));
        }
        Ok(())
    }

    fn enter_phase(&mut self, target: GeometryCursorPhase) -> NodeSceneSemanticResult<()> {
        if target < self.phase {
            return Err(invariant(
                "geometry cursor cannot return to an earlier table phase",
            ));
        }
        if target >= GeometryCursorPhase::Primitives
            && self.next_logical_mesh != self.proof.logical_mesh_count()
        {
            return Err(invariant(
                "logical-mesh table must be exhausted before primitives",
            ));
        }
        if target >= GeometryCursorPhase::Attributes
            && self.next_primitive != self.proof.primitive_count()
        {
            return Err(invariant(
                "primitive table must be exhausted before attributes",
            ));
        }
        if target >= GeometryCursorPhase::MeshInstances
            && self.next_attribute != self.proof.attribute_count()
        {
            return Err(invariant(
                "attribute table must be exhausted before mesh instances",
            ));
        }
        self.phase = target;
        Ok(())
    }
}

/// One logical mesh and its exact primitive range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneGeometryLogicalMesh<'scene> {
    source_index: usize,
    name: Option<&'scene str>,
    primitive_offset: usize,
    primitive_count: usize,
}

impl<'scene> NodeSceneGeometryLogicalMesh<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn primitive_offset(self) -> usize {
        self.primitive_offset
    }

    pub(in crate::canonical_scene::store) const fn name(self) -> Option<&'scene str> {
        self.name
    }

    pub(in crate::canonical_scene::store) const fn primitive_count(self) -> usize {
        self.primitive_count
    }
}

/// One primitive row with lazily resolved, presence-preserving payload leaves.
#[must_use = "the primitive payload cursors must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneGeometryPrimitive<'scene> {
    source_index: usize,
    topology: TopologyKind,
    vertex_count: u32,
    corner_count: u32,
    face_count: u32,
    polygon_boundaries_present: bool,
    polygon_boundary_count: usize,
    polygon_boundaries: Option<NodeSceneGeometryU32Cursor<'scene>>,
    indices_present: bool,
    index_count: usize,
    indices: Option<NodeSceneGeometryU32Cursor<'scene>>,
    material: Option<usize>,
    attribute_offset: usize,
    attribute_count: usize,
}

impl<'scene> NodeSceneGeometryPrimitive<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(&self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn topology(&self) -> TopologyKind {
        self.topology
    }

    pub(in crate::canonical_scene::store) const fn vertex_count(&self) -> u32 {
        self.vertex_count
    }

    pub(in crate::canonical_scene::store) const fn corner_count(&self) -> u32 {
        self.corner_count
    }

    pub(in crate::canonical_scene::store) const fn face_count(&self) -> u32 {
        self.face_count
    }

    pub(in crate::canonical_scene::store) const fn polygon_boundaries_present(&self) -> bool {
        self.polygon_boundaries_present
    }

    pub(in crate::canonical_scene::store) const fn polygon_boundary_count(&self) -> usize {
        self.polygon_boundary_count
    }

    pub(in crate::canonical_scene::store) fn take_polygon_boundaries(
        &mut self,
    ) -> Option<NodeSceneGeometryU32Cursor<'scene>> {
        self.polygon_boundaries.take()
    }

    pub(in crate::canonical_scene::store) const fn indices_present(&self) -> bool {
        self.indices_present
    }

    pub(in crate::canonical_scene::store) const fn index_count(&self) -> usize {
        self.index_count
    }

    pub(in crate::canonical_scene::store) fn take_indices(
        &mut self,
    ) -> Option<NodeSceneGeometryU32Cursor<'scene>> {
        self.indices.take()
    }

    pub(in crate::canonical_scene::store) const fn material(&self) -> Option<usize> {
        self.material
    }

    pub(in crate::canonical_scene::store) const fn attribute_offset(&self) -> usize {
        self.attribute_offset
    }

    pub(in crate::canonical_scene::store) const fn attribute_count(&self) -> usize {
        self.attribute_count
    }
}

/// Allocation-free built-in semantic decoded from the canonical wire code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneGeometryAttributeSemantic<'scene> {
    Position,
    Normal,
    Tangent,
    Texcoord(u32),
    Color(u32),
    SmoothingGroup,
    Custom(&'scene str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneGeometryScalarKind {
    F32,
    U32,
    I32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneGeometryNormalization {
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneGeometryPayloadKind {
    Dense,
    Sparse,
}

/// One attribute descriptor coupled to its exact typed payload cursors.
#[must_use = "the attribute payload cursors must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneGeometryAttribute<'scene> {
    source_index: usize,
    semantic: NodeSceneGeometryAttributeSemantic<'scene>,
    domain: AttributeDomain,
    scalar_kind: NodeSceneGeometryScalarKind,
    lanes: u8,
    normalization: NodeSceneGeometryNormalization,
    payload_kind: NodeSceneGeometryPayloadKind,
    element_count: u32,
    explicit_element_count: u32,
    canonical_scalar_count: u64,
    payload: NodeSceneGeometryAttributePayload<'scene>,
}

impl<'scene> NodeSceneGeometryAttribute<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(&self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn semantic(
        &self,
    ) -> NodeSceneGeometryAttributeSemantic<'scene> {
        self.semantic
    }

    pub(in crate::canonical_scene::store) const fn domain(&self) -> AttributeDomain {
        self.domain
    }

    pub(in crate::canonical_scene::store) const fn scalar_kind(
        &self,
    ) -> NodeSceneGeometryScalarKind {
        self.scalar_kind
    }

    pub(in crate::canonical_scene::store) const fn lanes(&self) -> u8 {
        self.lanes
    }

    pub(in crate::canonical_scene::store) const fn normalization(
        &self,
    ) -> NodeSceneGeometryNormalization {
        self.normalization
    }

    pub(in crate::canonical_scene::store) const fn payload_kind(
        &self,
    ) -> NodeSceneGeometryPayloadKind {
        self.payload_kind
    }

    pub(in crate::canonical_scene::store) const fn element_count(&self) -> u32 {
        self.element_count
    }

    pub(in crate::canonical_scene::store) const fn explicit_element_count(&self) -> u32 {
        self.explicit_element_count
    }

    pub(in crate::canonical_scene::store) const fn canonical_scalar_count(&self) -> u64 {
        self.canonical_scalar_count
    }

    pub(in crate::canonical_scene::store) fn into_payload(
        self,
    ) -> NodeSceneGeometryAttributePayload<'scene> {
        self.payload
    }
}

/// Typed attribute payload; empty cursors remain present for authored empty data.
#[must_use = "every payload leaf cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) enum NodeSceneGeometryAttributePayload<'scene> {
    DenseF32 {
        values: NodeSceneGeometryF32Cursor<'scene>,
    },
    DenseU32 {
        values: NodeSceneGeometryU32Cursor<'scene>,
    },
    DenseI32 {
        values: NodeSceneGeometryI32Cursor<'scene>,
    },
    SparseF32 {
        values: NodeSceneGeometryF32Cursor<'scene>,
        indices: NodeSceneGeometryU32Cursor<'scene>,
        default: NodeSceneGeometryF32Cursor<'scene>,
    },
    SparseU32 {
        values: NodeSceneGeometryU32Cursor<'scene>,
        indices: NodeSceneGeometryU32Cursor<'scene>,
        default: NodeSceneGeometryU32Cursor<'scene>,
    },
    SparseI32 {
        values: NodeSceneGeometryI32Cursor<'scene>,
        indices: NodeSceneGeometryU32Cursor<'scene>,
        default: NodeSceneGeometryI32Cursor<'scene>,
    },
}

/// One source-ordered mesh instance with canonical source-space references.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneGeometryMeshInstance {
    source_index: usize,
    owner_node: usize,
    logical_mesh: usize,
}

impl NodeSceneGeometryMeshInstance {
    pub(in crate::canonical_scene::store) const fn source_index(self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn owner_node(self) -> usize {
        self.owner_node
    }

    pub(in crate::canonical_scene::store) const fn logical_mesh(self) -> usize {
        self.logical_mesh
    }
}

/// Exact, allocation-free traversal of one typed scalar-sequence leaf.
#[must_use = "the payload leaf cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneGeometryLeafCursor<'scene, T: StoredScalar> {
    values: &'scene [T],
    next: usize,
}

pub(in crate::canonical_scene::store) type NodeSceneGeometryF32Cursor<'scene> =
    NodeSceneGeometryLeafCursor<'scene, f32>;
pub(in crate::canonical_scene::store) type NodeSceneGeometryU32Cursor<'scene> =
    NodeSceneGeometryLeafCursor<'scene, u32>;
pub(in crate::canonical_scene::store) type NodeSceneGeometryI32Cursor<'scene> =
    NodeSceneGeometryLeafCursor<'scene, i32>;

impl<'scene, T: StoredScalar> NodeSceneGeometryLeafCursor<'scene, T> {
    const fn new(values: &'scene [T]) -> Self {
        Self { values, next: 0 }
    }

    pub(in crate::canonical_scene::store) const fn declared_count(&self) -> usize {
        self.values.len()
    }

    pub(in crate::canonical_scene::store) fn next(&mut self) -> NodeSceneSemanticResult<Option<T>> {
        if self.next == self.values.len() {
            return Ok(None);
        }
        let value =
            self.values.get(self.next).copied().ok_or_else(|| {
                invariant("payload leaf cursor advanced beyond its declared count")
            })?;
        self.next = advance_row(self.next, "payload leaf cursor index overflows")?;
        Ok(Some(value))
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        if self.next != self.values.len() {
            return Err(invariant(
                "payload leaf cursor ended before its declared count",
            ));
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn read_attribute_payload<'scene>(
    canonical: &'scene SceneCanonicalStore,
    table: CanonicalRecordView<'scene>,
    row: usize,
    scalar_kind: NodeSceneGeometryScalarKind,
    payload_kind: NodeSceneGeometryPayloadKind,
    lanes: u8,
    element_count: u32,
    explicit_element_count: u32,
    canonical_scalar_count: u64,
) -> NodeSceneSemanticResult<NodeSceneGeometryAttributePayload<'scene>> {
    let lane_count = usize::from(lanes);
    let element_count = usize::try_from(element_count)
        .map_err(|_| invariant("validated attribute element count cannot address memory"))?;
    let explicit_element_count = usize::try_from(explicit_element_count).map_err(|_| {
        invariant("validated attribute explicit-element count cannot address memory")
    })?;
    let value_count = match payload_kind {
        NodeSceneGeometryPayloadKind::Dense => {
            if explicit_element_count != element_count {
                return Err(invariant(
                    "validated dense attribute explicit count disagrees with element count",
                ));
            }
            element_count
                .checked_mul(lane_count)
                .ok_or_else(|| invariant("validated dense attribute scalar count overflows"))?
        }
        NodeSceneGeometryPayloadKind::Sparse => {
            if explicit_element_count > element_count {
                return Err(invariant(
                    "validated sparse attribute explicit count exceeds element count",
                ));
            }
            explicit_element_count
                .checked_mul(lane_count)
                .ok_or_else(|| invariant("validated sparse attribute scalar count overflows"))?
        }
    };
    let expected_canonical_count = match payload_kind {
        NodeSceneGeometryPayloadKind::Dense => value_count,
        NodeSceneGeometryPayloadKind::Sparse => value_count
            .checked_add(lane_count)
            .ok_or_else(|| invariant("validated sparse canonical scalar count overflows"))?,
    };
    if u64::try_from(expected_canonical_count).ok() != Some(canonical_scalar_count) {
        return Err(invariant(
            "validated attribute payload shape disagrees with canonical scalar count",
        ));
    }

    match (scalar_kind, payload_kind) {
        (NodeSceneGeometryScalarKind::F32, NodeSceneGeometryPayloadKind::Dense) => {
            let values = read_f32_leaf::<GeometryF32ValuesSchema>(
                canonical,
                table,
                row,
                geometry_attribute_field::VALUES,
                value_count,
                "validated dense f32 values are unavailable or mistyped",
            )?;
            require_absent_cell(
                table,
                row,
                geometry_attribute_field::SPARSE_INDICES,
                "validated dense attribute retains sparse indices",
            )?;
            require_absent_cell(
                table,
                row,
                geometry_attribute_field::SPARSE_DEFAULT,
                "validated dense attribute retains a sparse default",
            )?;
            Ok(NodeSceneGeometryAttributePayload::DenseF32 { values })
        }
        (NodeSceneGeometryScalarKind::U32, NodeSceneGeometryPayloadKind::Dense) => {
            let values = read_u32_leaf::<GeometryU32ValuesSchema>(
                canonical,
                table,
                row,
                geometry_attribute_field::VALUES,
                value_count,
                "validated dense u32 values are unavailable or mistyped",
            )?;
            require_absent_cell(
                table,
                row,
                geometry_attribute_field::SPARSE_INDICES,
                "validated dense attribute retains sparse indices",
            )?;
            require_absent_cell(
                table,
                row,
                geometry_attribute_field::SPARSE_DEFAULT,
                "validated dense attribute retains a sparse default",
            )?;
            Ok(NodeSceneGeometryAttributePayload::DenseU32 { values })
        }
        (NodeSceneGeometryScalarKind::I32, NodeSceneGeometryPayloadKind::Dense) => {
            let values = read_i32_leaf::<GeometryI32ValuesSchema>(
                canonical,
                table,
                row,
                geometry_attribute_field::VALUES,
                value_count,
                "validated dense i32 values are unavailable or mistyped",
            )?;
            require_absent_cell(
                table,
                row,
                geometry_attribute_field::SPARSE_INDICES,
                "validated dense attribute retains sparse indices",
            )?;
            require_absent_cell(
                table,
                row,
                geometry_attribute_field::SPARSE_DEFAULT,
                "validated dense attribute retains a sparse default",
            )?;
            Ok(NodeSceneGeometryAttributePayload::DenseI32 { values })
        }
        (NodeSceneGeometryScalarKind::F32, NodeSceneGeometryPayloadKind::Sparse) => {
            let values = read_f32_leaf::<GeometryF32ValuesSchema>(
                canonical,
                table,
                row,
                geometry_attribute_field::VALUES,
                value_count,
                "validated sparse f32 values are unavailable or mistyped",
            )?;
            let indices = read_u32_leaf::<GeometrySparseIndicesSchema>(
                canonical,
                table,
                row,
                geometry_attribute_field::SPARSE_INDICES,
                explicit_element_count,
                "validated sparse indices are unavailable or mistyped",
            )?;
            let default = read_f32_leaf::<GeometrySparseF32DefaultsSchema>(
                canonical,
                table,
                row,
                geometry_attribute_field::SPARSE_DEFAULT,
                lane_count,
                "validated sparse f32 default is unavailable or mistyped",
            )?;
            Ok(NodeSceneGeometryAttributePayload::SparseF32 {
                values,
                indices,
                default,
            })
        }
        (NodeSceneGeometryScalarKind::U32, NodeSceneGeometryPayloadKind::Sparse) => {
            let values = read_u32_leaf::<GeometryU32ValuesSchema>(
                canonical,
                table,
                row,
                geometry_attribute_field::VALUES,
                value_count,
                "validated sparse u32 values are unavailable or mistyped",
            )?;
            let indices = read_u32_leaf::<GeometrySparseIndicesSchema>(
                canonical,
                table,
                row,
                geometry_attribute_field::SPARSE_INDICES,
                explicit_element_count,
                "validated sparse indices are unavailable or mistyped",
            )?;
            let default = read_u32_leaf::<GeometrySparseU32DefaultsSchema>(
                canonical,
                table,
                row,
                geometry_attribute_field::SPARSE_DEFAULT,
                lane_count,
                "validated sparse u32 default is unavailable or mistyped",
            )?;
            Ok(NodeSceneGeometryAttributePayload::SparseU32 {
                values,
                indices,
                default,
            })
        }
        (NodeSceneGeometryScalarKind::I32, NodeSceneGeometryPayloadKind::Sparse) => {
            let values = read_i32_leaf::<GeometryI32ValuesSchema>(
                canonical,
                table,
                row,
                geometry_attribute_field::VALUES,
                value_count,
                "validated sparse i32 values are unavailable or mistyped",
            )?;
            let indices = read_u32_leaf::<GeometrySparseIndicesSchema>(
                canonical,
                table,
                row,
                geometry_attribute_field::SPARSE_INDICES,
                explicit_element_count,
                "validated sparse indices are unavailable or mistyped",
            )?;
            let default = read_i32_leaf::<GeometrySparseI32DefaultsSchema>(
                canonical,
                table,
                row,
                geometry_attribute_field::SPARSE_DEFAULT,
                lane_count,
                "validated sparse i32 default is unavailable or mistyped",
            )?;
            Ok(NodeSceneGeometryAttributePayload::SparseI32 {
                values,
                indices,
                default,
            })
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn read_optional_u32_leaf<'scene, Tag>(
    canonical: &'scene SceneCanonicalStore,
    table: CanonicalRecordView<'scene>,
    row: usize,
    presence_field: usize,
    count_field: usize,
    reference_field: usize,
    presence_reason: &'static str,
    count_reason: &'static str,
    payload_reason: &'static str,
) -> NodeSceneSemanticResult<Option<NodeSceneGeometryU32Cursor<'scene>>>
where
    Tag: Copy + Send + Sync + 'static,
{
    let presence = read_scalar::<u8>(table, row, presence_field, presence_reason)?;
    let count = read_u32_as_usize(table, row, count_field, count_reason)?;
    match presence {
        PRESENCE_ABSENT => {
            if count != 0 {
                return Err(invariant(
                    "validated absent payload has a nonzero declared count",
                ));
            }
            require_absent_cell(table, row, reference_field, payload_reason)?;
            Ok(None)
        }
        PRESENCE_PRESENT => read_u32_leaf::<Tag>(
            canonical,
            table,
            row,
            reference_field,
            count,
            payload_reason,
        )
        .map(Some),
        _ => Err(invariant("validated payload presence code is unknown")),
    }
}

fn read_f32_leaf<'scene, Tag>(
    canonical: &'scene SceneCanonicalStore,
    table: CanonicalRecordView<'scene>,
    row: usize,
    field: usize,
    expected_count: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<NodeSceneGeometryF32Cursor<'scene>>
where
    Tag: Copy + Send + Sync + 'static,
{
    let values = resolve_leaf::<f32, Tag>(canonical, table, row, field, expected_count, reason)?;
    Ok(NodeSceneGeometryLeafCursor::new(values))
}

fn read_u32_leaf<'scene, Tag>(
    canonical: &'scene SceneCanonicalStore,
    table: CanonicalRecordView<'scene>,
    row: usize,
    field: usize,
    expected_count: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<NodeSceneGeometryU32Cursor<'scene>>
where
    Tag: Copy + Send + Sync + 'static,
{
    let values = resolve_leaf::<u32, Tag>(canonical, table, row, field, expected_count, reason)?;
    Ok(NodeSceneGeometryLeafCursor::new(values))
}

fn read_i32_leaf<'scene, Tag>(
    canonical: &'scene SceneCanonicalStore,
    table: CanonicalRecordView<'scene>,
    row: usize,
    field: usize,
    expected_count: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<NodeSceneGeometryI32Cursor<'scene>>
where
    Tag: Copy + Send + Sync + 'static,
{
    let values = resolve_leaf::<i32, Tag>(canonical, table, row, field, expected_count, reason)?;
    Ok(NodeSceneGeometryLeafCursor::new(values))
}

fn resolve_leaf<'scene, T, Tag>(
    canonical: &'scene SceneCanonicalStore,
    table: CanonicalRecordView<'scene>,
    row: usize,
    field: usize,
    expected_count: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<&'scene [T]>
where
    T: StoredScalar,
    Tag: Copy + Send + Sync + 'static,
{
    if expected_count == 0 {
        require_absent_cell(table, row, field, reason)?;
        return Ok(&[]);
    }
    let reference = table
        .sequence_ref::<T, Tag>(row, field)
        .ok_or_else(|| invariant(reason))?;
    let sequence: SequenceView<'scene, T> = canonical
        .sequence(reference)
        .ok_or_else(|| invariant(reason))?;
    let values = sequence.as_slice();
    if values.len() != expected_count {
        return Err(invariant(
            "validated payload leaf length disagrees with its declared count",
        ));
    }
    Ok(values)
}

fn read_optional_material(
    table: CanonicalRecordView<'_>,
    row: usize,
    material_count: usize,
    materials: Option<CanonicalBlockRef<AppearanceMaterialTableSchema>>,
) -> NodeSceneSemanticResult<Option<usize>> {
    let presence = read_scalar::<u8>(
        table,
        row,
        geometry_primitive_field::MATERIAL_PRESENCE,
        "validated primitive material presence is unreadable",
    )?;
    match presence {
        PRESENCE_ABSENT => {
            let material = read_scalar::<u32>(
                table,
                row,
                geometry_primitive_field::MATERIAL,
                "validated absent primitive material is unreadable",
            )?;
            if material != 0 {
                return Err(invariant(
                    "validated absent primitive material has a nonzero identity",
                ));
            }
            require_absent_cell(
                table,
                row,
                geometry_primitive_field::MATERIALS,
                "validated absent primitive material retains an appearance-material table reference",
            )?;
            Ok(None)
        }
        PRESENCE_PRESENT => {
            let material = read_u32_as_usize(
                table,
                row,
                geometry_primitive_field::MATERIAL,
                "validated primitive material identity is unreadable",
            )?;
            if material >= material_count {
                return Err(invariant(
                    "validated primitive material is outside appearance material space",
                ));
            }
            let materials = materials.ok_or_else(|| {
                invariant("validated primitive appearance-material table proof is absent")
            })?;
            require_exact_record_reference(
                table,
                row,
                geometry_primitive_field::MATERIALS,
                materials,
                "validated primitive appearance-material table reference is unreadable",
            )?;
            Ok(Some(material))
        }
        _ => Err(invariant(
            "validated primitive material presence code is unknown",
        )),
    }
}

fn require_record_reference<Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    expected: Option<CanonicalBlockRef<Tag>>,
    range_count: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<()>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    if range_count == 0 {
        return require_absent_cell(table, row, field, reason);
    }
    let expected = expected.ok_or_else(|| invariant(reason))?;
    require_exact_record_reference(table, row, field, expected, reason)
}

fn require_exact_record_reference<Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    expected: CanonicalBlockRef<Tag>,
    reason: &'static str,
) -> NodeSceneSemanticResult<()>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    let actual = table
        .record_ref::<Tag>(row, field)
        .ok_or_else(|| invariant(reason))?;
    if actual != expected.into_store() {
        return Err(invariant(reason));
    }
    Ok(())
}

fn require_utf8_ref<Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    expected: Option<CanonicalBlockRef<Tag>>,
    unreadable_reason: &'static str,
    mismatch_reason: &'static str,
) -> NodeSceneSemanticResult<()>
where
    Tag: CanonicalSchema<StoreKind = Utf8Block>,
{
    let reference = table
        .utf8_ref::<Tag>(row, field)
        .ok_or_else(|| invariant(unreadable_reason))?;
    if Some(CanonicalBlockRef::from_store(reference)) != expected {
        return Err(invariant(mismatch_reason));
    }
    Ok(())
}

fn require_absent_cell(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<()> {
    match table.scalar::<u8>(row, field) {
        Some(0) => Ok(()),
        _ => Err(invariant(reason)),
    }
}

fn require_row_ordinal(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    unreadable_reason: &'static str,
    mismatch_reason: &'static str,
) -> NodeSceneSemanticResult<()> {
    let ordinal = read_u32_as_usize(table, row, field, unreadable_reason)?;
    if ordinal != row {
        return Err(invariant(mismatch_reason));
    }
    Ok(())
}

fn require_range(
    offset: usize,
    count: usize,
    total: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<()> {
    let end = offset.checked_add(count).ok_or_else(|| invariant(reason))?;
    if end > total {
        return Err(invariant(reason));
    }
    Ok(())
}

fn next_row(
    next: usize,
    total: usize,
    overflow_reason: &'static str,
) -> NodeSceneSemanticResult<Option<usize>> {
    if next == total {
        return Ok(None);
    }
    if next > total {
        return Err(invariant(overflow_reason));
    }
    Ok(Some(next))
}

fn advance_row(row: usize, reason: &'static str) -> NodeSceneSemanticResult<usize> {
    row.checked_add(1).ok_or_else(|| invariant(reason))
}

fn decode_topology(value: u8) -> NodeSceneSemanticResult<TopologyKind> {
    match value {
        GEOMETRY_TOPOLOGY_POINTS => Ok(TopologyKind::Points),
        GEOMETRY_TOPOLOGY_LINE_LIST => Ok(TopologyKind::LineList),
        GEOMETRY_TOPOLOGY_LINE_STRIP => Ok(TopologyKind::LineStrip),
        GEOMETRY_TOPOLOGY_LINE_LOOP => Ok(TopologyKind::LineLoop),
        GEOMETRY_TOPOLOGY_TRIANGLE_LIST => Ok(TopologyKind::TriangleList),
        GEOMETRY_TOPOLOGY_TRIANGLE_STRIP => Ok(TopologyKind::TriangleStrip),
        GEOMETRY_TOPOLOGY_TRIANGLE_FAN => Ok(TopologyKind::TriangleFan),
        GEOMETRY_TOPOLOGY_POLYGONS => Ok(TopologyKind::Polygons),
        _ => Err(invariant("validated primitive topology code is unknown")),
    }
}

fn decode_semantic<'scene>(
    kind: u8,
    set: u32,
    custom_name: Option<&'scene str>,
) -> NodeSceneSemanticResult<NodeSceneGeometryAttributeSemantic<'scene>> {
    match kind {
        GEOMETRY_SEMANTIC_POSITION if set == 0 && custom_name.is_none() => {
            Ok(NodeSceneGeometryAttributeSemantic::Position)
        }
        GEOMETRY_SEMANTIC_NORMAL if set == 0 && custom_name.is_none() => {
            Ok(NodeSceneGeometryAttributeSemantic::Normal)
        }
        GEOMETRY_SEMANTIC_TANGENT if set == 0 && custom_name.is_none() => {
            Ok(NodeSceneGeometryAttributeSemantic::Tangent)
        }
        GEOMETRY_SEMANTIC_TEXCOORD if custom_name.is_none() => {
            Ok(NodeSceneGeometryAttributeSemantic::Texcoord(set))
        }
        GEOMETRY_SEMANTIC_COLOR if custom_name.is_none() => {
            Ok(NodeSceneGeometryAttributeSemantic::Color(set))
        }
        GEOMETRY_SEMANTIC_SMOOTHING_GROUP if set == 0 => {
            Ok(NodeSceneGeometryAttributeSemantic::SmoothingGroup)
        }
        GEOMETRY_SEMANTIC_CUSTOM if set == 0 => custom_name
            .map(NodeSceneGeometryAttributeSemantic::Custom)
            .ok_or_else(|| invariant("validated custom attribute semantic has no name")),
        GEOMETRY_SEMANTIC_POSITION
        | GEOMETRY_SEMANTIC_NORMAL
        | GEOMETRY_SEMANTIC_TANGENT
        | GEOMETRY_SEMANTIC_SMOOTHING_GROUP => Err(invariant(
            "validated non-indexed attribute semantic has a nonzero set",
        )),
        _ => Err(invariant("validated attribute semantic code is unknown")),
    }
}

fn decode_domain(value: u8) -> NodeSceneSemanticResult<AttributeDomain> {
    match value {
        GEOMETRY_DOMAIN_VERTEX => Ok(AttributeDomain::Vertex),
        GEOMETRY_DOMAIN_CORNER => Ok(AttributeDomain::Corner),
        GEOMETRY_DOMAIN_FACE => Ok(AttributeDomain::Face),
        _ => Err(invariant("validated attribute domain code is unknown")),
    }
}

fn decode_scalar_kind(value: u8) -> NodeSceneSemanticResult<NodeSceneGeometryScalarKind> {
    match value {
        GEOMETRY_SCALAR_F32 => Ok(NodeSceneGeometryScalarKind::F32),
        GEOMETRY_SCALAR_U32 => Ok(NodeSceneGeometryScalarKind::U32),
        GEOMETRY_SCALAR_I32 => Ok(NodeSceneGeometryScalarKind::I32),
        _ => Err(invariant("validated attribute scalar-kind code is unknown")),
    }
}

fn decode_normalization(value: u8) -> NodeSceneSemanticResult<NodeSceneGeometryNormalization> {
    match value {
        GEOMETRY_NORMALIZATION_NONE => Ok(NodeSceneGeometryNormalization::None),
        _ => Err(invariant(
            "validated attribute normalization code is unknown",
        )),
    }
}

fn decode_payload_kind(value: u8) -> NodeSceneSemanticResult<NodeSceneGeometryPayloadKind> {
    match value {
        GEOMETRY_PAYLOAD_DENSE => Ok(NodeSceneGeometryPayloadKind::Dense),
        GEOMETRY_PAYLOAD_SPARSE => Ok(NodeSceneGeometryPayloadKind::Sparse),
        _ => Err(invariant(
            "validated attribute payload-kind code is unknown",
        )),
    }
}
