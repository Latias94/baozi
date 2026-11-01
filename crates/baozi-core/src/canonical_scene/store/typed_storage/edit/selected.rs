//! A source-bound selection, reconstructed with one streaming walk of the geometry owner.

#[cfg(test)]
mod tests;

mod storage;
pub use storage::{SceneGeometryPlanScratch, SceneGeometryPlanStorage};

mod writer;
use writer::{Mode, NameKind, Writer};

use baozi_operation::CapacityPlan;

use crate::attribute::AttributeDomain;
use crate::geometry::TopologyKind;
use crate::identity::PrimitiveId;

use super::super::validation::TypedStorageValidationControl;
use super::super::{
    StoredAttributeSemantic as Semantic, StoredTangentBasis, StoredTangentConventions,
    StoredTangentEncoding, TextRange,
};
use super::{
    EditResult, GeometryStorage, StoredAttribute, StoredAttributePayload as Payload,
    StoredLogicalMesh, StoredMeshInstance, StoredPrimitive, TableRange, TypedStorageBuildControl,
    TypedStorageBuildError, VectorValues, allocate, capacity, overflow,
};

/// Exact source-corner triples and source-face prefix offsets. Strip/fan assembly is not rotated.
#[derive(Debug, Clone, Copy)]
pub struct SceneTriangleOrigins<'a> {
    pub triangles: &'a [[u32; 3]],
    pub face_offsets: &'a [u32],
}

/// Primitive-local provisional output. Only the common transaction can resolve its source rows.
pub struct SceneGeometryReplacement<'a> {
    pub target: PrimitiveId,
    pub origins: Option<SceneTriangleOrigins<'a>>,
    pub normals: Option<VectorValues<'a, 3>>,
    pub tangents: Option<VectorValues<'a, 4>>,
    pub remove_tangents: bool,
    pub smooth: bool,
    pub normal_domain: AttributeDomain,
    pub uv_set: u32,
    pub uv_domain: AttributeDomain,
}

impl SceneGeometryReplacement<'_> {
    pub const fn new(target: PrimitiveId) -> Self {
        Self {
            target,
            origins: None,
            normals: None,
            tangents: None,
            remove_tangents: false,
            smooth: false,
            normal_domain: AttributeDomain::Corner,
            uv_set: 0,
            uv_domain: AttributeDomain::Vertex,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct LocalMapping {
    pub row: usize,
    pub source_start: usize,
    pub source_count: usize,
    pub output_start: usize,
    pub output_count: usize,
    pub position: Option<u32>,
    pub normal: Option<u32>,
    pub tangent: Option<u32>,
    pub uv: Option<u32>,
    pub smoothing: Option<u32>,
    pub output_normal: Option<u32>,
    pub output_tangent: Option<u32>,
    pub corner_offset: usize,
    pub face_offset: usize,
}

impl LocalMapping {
    pub(super) const EMPTY: Self = Self {
        row: 0,
        source_start: 0,
        source_count: 0,
        output_start: 0,
        output_count: 0,
        position: None,
        normal: None,
        tangent: None,
        uv: None,
        smoothing: None,
        output_normal: None,
        output_tangent: None,
        corner_offset: 0,
        face_offset: 0,
    };
}

/// No per-target GeometryStorage, whole-Scene intermediate, or per-target full-source plan exists.
#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct GeometrySelectionPlan<'a> {
    source: &'a GeometryStorage,
    edits: &'a [SceneGeometryReplacement<'a>],
    mappings: &'a [LocalMapping],
    capacities: [CapacityPlan; 9],
    work: u64,
    validation_work: u64,
    changes_fields: bool,
}

impl<'a> GeometrySelectionPlan<'a> {
    pub(in crate::canonical_scene::store) fn try_new<C: TypedStorageBuildControl>(
        source: &'a GeometryStorage,
        edits: &'a [SceneGeometryReplacement<'a>],
        scratch: SceneGeometryPlanScratch<'a>,
        control: &mut C,
    ) -> EditResult<Self, C::Error> {
        let mappings = scratch.initialize(edits.len(), control)?;
        let mut previous = None;
        let mut corner_offset = 0_usize;
        let mut face_offset = 0_usize;
        for (index, edit) in edits.iter().enumerate() {
            let row = control
                .run_unit(|| {
                    let mesh = source
                        .logical_meshes
                        .get(edit.target.logical_mesh().index() as usize)?;
                    ((edit.target.local() as usize) < mesh.primitives.len())
                        .then(|| mesh.primitives.start() + edit.target.local() as usize)
                })
                .map_err(TypedStorageBuildError::Control)?
                .ok_or(TypedStorageBuildError::Invariant)?;
            if previous.is_some_and(|old| old >= row) {
                return Err(TypedStorageBuildError::Invariant);
            }
            previous = Some(row);
            let mapping = LocalMapping {
                row,
                source_start: 0,
                source_count: 0,
                output_start: 0,
                output_count: 0,
                position: None,
                normal: None,
                tangent: None,
                uv: None,
                smoothing: None,
                output_normal: None,
                output_tangent: None,
                corner_offset,
                face_offset,
            };
            validate_local(source, edit, row, control)?;
            control
                .run_unit(|| mappings[index] = mapping)
                .map_err(TypedStorageBuildError::Control)?;
            if let Some(o) = edit.origins {
                corner_offset = corner_offset
                    .checked_add(o.triangles.len().checked_mul(3).ok_or_else(overflow)?)
                    .ok_or_else(overflow)?;
                face_offset = face_offset
                    .checked_add(o.face_offsets.len())
                    .ok_or_else(overflow)?;
            }
        }
        let (counts, work) = {
            let mut writer = Writer::new(Mode::Count, control);
            reconstruct(
                source,
                edits,
                MappingPass::Planning(&mut *mappings),
                &mut writer,
            )?;
            (writer.counts, writer.work)
        };
        u32::try_from(counts[2]).map_err(|_| overflow())?;
        let mut changes_fields = false;
        for (edit, mapping) in edits.iter().zip(mappings.iter()) {
            changes_fields |= control
                .run_unit(|| {
                    edit.normals.is_some()
                        || edit.tangents.is_some()
                        || edit.remove_tangents && mapping.tangent.is_some()
                })
                .map_err(TypedStorageBuildError::Control)?;
        }
        let capacities = [
            capacity::<StoredLogicalMesh, C::Error>(counts[0])?,
            capacity::<StoredPrimitive, C::Error>(counts[1])?,
            capacity::<StoredAttribute, C::Error>(counts[2])?,
            capacity::<StoredMeshInstance, C::Error>(counts[3])?,
            capacity::<u8, C::Error>(counts[4])?,
            capacity::<f32, C::Error>(counts[5])?,
            capacity::<f64, C::Error>(counts[6])?,
            capacity::<u32, C::Error>(counts[7])?,
            capacity::<i32, C::Error>(counts[8])?,
        ];
        let validation_work = [
            counts[0], counts[1], counts[1], counts[2], counts[3], counts[5], counts[6], counts[7],
            counts[8],
        ]
        .into_iter()
        .try_fold(0_u64, |sum, n| {
            sum.checked_add(n as u64).ok_or_else(overflow)
        })?;
        Ok(Self {
            source,
            edits,
            mappings,
            capacities,
            work,
            validation_work,
            changes_fields,
        })
    }

    pub(in crate::canonical_scene::store) const fn capacities(self) -> [CapacityPlan; 9] {
        self.capacities
    }
    pub(in crate::canonical_scene::store) const fn work(self) -> u64 {
        self.work
    }
    pub(in crate::canonical_scene::store) const fn correspondence_work(self) -> u64 {
        self.work
    }
    pub(in crate::canonical_scene::store) const fn validation_work(self) -> u64 {
        self.validation_work
    }
    pub(in crate::canonical_scene::store) const fn changes_fields(self) -> bool {
        self.changes_fields
    }
    pub(in crate::canonical_scene::store) fn name_bytes(self) -> usize {
        self.capacities[4].desired_capacity()
    }
    pub(in crate::canonical_scene::store) fn output_attribute_count(self) -> u32 {
        self.capacities[2].desired_capacity() as u32
    }
    pub(in crate::canonical_scene::store) const fn source_geometry(self) -> &'a GeometryStorage {
        self.source
    }
    pub(in crate::canonical_scene::store) const fn edits(
        self,
    ) -> &'a [SceneGeometryReplacement<'a>] {
        self.edits
    }
    pub(in crate::canonical_scene::store) const fn mappings(self) -> &'a [LocalMapping] {
        self.mappings
    }
    pub(in crate::canonical_scene::store) fn mapping(self, index: usize) -> LocalMapping {
        self.mappings[index]
    }
    pub(in crate::canonical_scene::store) fn changed(self, index: usize) -> bool {
        let edit = &self.edits[index];
        edit.origins.is_some()
            || edit.normals.is_some()
            || edit.tangents.is_some()
            || edit.remove_tangents && self.mappings[index].tangent.is_some()
    }

    pub(in crate::canonical_scene::store) fn materialize<C: TypedStorageBuildControl>(
        self,
        control: &mut C,
    ) -> EditResult<GeometryStorage, C::Error> {
        let p = self.capacities;
        let mut output = GeometryStorage {
            logical_meshes: allocate(p[0])?,
            primitives: allocate(p[1])?,
            attributes: allocate(p[2])?,
            mesh_instances: allocate(p[3])?,
            names: allocate(p[4])?,
            f32_values: allocate(p[5])?,
            f64_values: allocate(p[6])?,
            u32_values: allocate(p[7])?,
            i32_values: allocate(p[8])?,
        };
        {
            let mut writer = Writer::new(Mode::Build(&mut output), control);
            reconstruct(
                self.source,
                self.edits,
                MappingPass::Sealed(self.mappings),
                &mut writer,
            )?;
            if writer.counts != p.map(|p| p.desired_capacity()) || writer.work != self.work {
                return Err(TypedStorageBuildError::Invariant);
            }
        }
        for (actual, plan) in [
            output.logical_meshes.capacity(),
            output.primitives.capacity(),
            output.attributes.capacity(),
            output.mesh_instances.capacity(),
            output.names.capacity(),
            output.f32_values.capacity(),
            output.f64_values.capacity(),
            output.u32_values.capacity(),
            output.i32_values.capacity(),
        ]
        .into_iter()
        .zip(p)
        {
            plan.finish(0, actual)
                .map_err(TypedStorageBuildError::Capacity)?;
        }
        Ok(output)
    }

    pub(in crate::canonical_scene::store) fn validate_correspondence<
        C: TypedStorageValidationControl,
    >(
        self,
        actual: &GeometryStorage,
        control: &mut C,
    ) -> Result<bool, C::Error> {
        if [
            actual.logical_meshes.len(),
            actual.primitives.len(),
            actual.attributes.len(),
            actual.mesh_instances.len(),
            actual.names.len(),
            actual.f32_values.len(),
            actual.f64_values.len(),
            actual.u32_values.len(),
            actual.i32_values.len(),
        ] != self.capacities.map(|p| p.desired_capacity())
        {
            return Ok(false);
        }
        struct Adapter<'a, C>(&'a mut C);
        impl<C: TypedStorageValidationControl> TypedStorageBuildControl for Adapter<'_, C> {
            type Error = Option<C::Error>;
            fn run_unit<R>(&mut self, f: impl FnOnce() -> R) -> Result<R, Self::Error> {
                self.0.run_unit(f).map_err(Some)?.ok_or(None)
            }
            fn run_string_unit<R>(&mut self, f: impl FnOnce() -> R) -> Result<R, Self::Error> {
                self.run_unit(f)
            }
        }
        let mut adapter = Adapter(control);
        let mut writer = Writer::new(Mode::Compare(actual), &mut adapter);
        match reconstruct(
            self.source,
            self.edits,
            MappingPass::Sealed(self.mappings),
            &mut writer,
        ) {
            Ok(()) => Ok(writer.work == self.work),
            Err(TypedStorageBuildError::Control(Some(error))) => Err(error),
            Err(_) => Ok(false),
        }
    }

    pub(in crate::canonical_scene::store) fn edit_for_primitive(
        self,
        row: usize,
    ) -> Option<&'a SceneGeometryReplacement<'a>> {
        self.mappings
            .binary_search_by_key(&row, |m| m.row)
            .ok()
            .filter(|&i| self.changed(i))
            .map(|i| &self.edits[i])
    }

    pub(in crate::canonical_scene::store) fn source_attribute(self, output: u32) -> Option<u32> {
        let i = self
            .mappings
            .partition_point(|m| m.output_start <= output as usize);
        let Some(index) = i.checked_sub(1) else {
            return Some(output);
        };
        let edit = &self.edits[index];
        let m = self.mapping(index);
        if output as usize >= m.output_start + m.output_count {
            return u32::try_from(
                output as usize - m.output_start - m.output_count + m.source_start + m.source_count,
            )
            .ok();
        }
        if m.output_normal == Some(output) && edit.normals.is_some() {
            return m.normal;
        }
        if m.output_tangent == Some(output) && edit.tangents.is_some() {
            return m.tangent;
        }
        let added_before =
            usize::from(edit.normals.is_some() && m.output_normal.is_some_and(|i| i < output))
                + usize::from(
                    edit.tangents.is_some() && m.output_tangent.is_some_and(|i| i < output),
                );
        let mut source =
            m.source_start + (output as usize - m.output_start).checked_sub(added_before)?;
        for removed in [
            m.normal.filter(|_| edit.normals.is_some()),
            m.tangent
                .filter(|_| edit.tangents.is_some() || edit.remove_tangents),
        ]
        .into_iter()
        .flatten()
        {
            if removed as usize <= source {
                source += 1;
            }
        }
        (source < m.source_start + m.source_count).then_some(source as u32)
    }

    pub(in crate::canonical_scene::store) fn output_attribute(self, source: u32) -> Option<u32> {
        let i = self
            .mappings
            .partition_point(|m| m.source_start <= source as usize);
        let Some(index) = i.checked_sub(1) else {
            return Some(source);
        };
        let edit = &self.edits[index];
        let m = self.mapping(index);
        if source as usize >= m.source_start + m.source_count {
            return u32::try_from(
                source as usize - m.source_start - m.source_count + m.output_start + m.output_count,
            )
            .ok();
        }
        if m.normal == Some(source) && edit.normals.is_some() {
            return m.output_normal;
        }
        if m.tangent == Some(source) && (edit.tangents.is_some() || edit.remove_tangents) {
            return if edit.tangents.is_some() {
                m.output_tangent
            } else {
                None
            };
        }
        let removed_before =
            usize::from(edit.normals.is_some() && m.normal.is_some_and(|i| i < source))
                + usize::from(
                    (edit.tangents.is_some() || edit.remove_tangents)
                        && m.tangent.is_some_and(|i| i < source),
                );
        let mut output =
            m.output_start + (source as usize - m.source_start).checked_sub(removed_before)?;
        for added in [
            m.output_normal.filter(|_| edit.normals.is_some()),
            m.output_tangent.filter(|_| edit.tangents.is_some()),
        ]
        .into_iter()
        .flatten()
        {
            if added as usize <= output {
                output += 1;
            }
        }
        (output < m.output_start + m.output_count).then_some(output as u32)
    }
}

fn validate_local<C: TypedStorageBuildControl>(
    source: &GeometryStorage,
    edit: &SceneGeometryReplacement<'_>,
    row: usize,
    control: &mut C,
) -> EditResult<(), C::Error> {
    let p = source
        .primitives
        .get(row)
        .ok_or(TypedStorageBuildError::Invariant)?;
    if edit.normals.is_some() && edit.tangents.is_none() && !edit.remove_tangents {
        for attribute in p
            .attributes
            .get(&source.attributes)
            .ok_or(TypedStorageBuildError::Invariant)?
        {
            if control
                .run_unit(|| attribute.semantic == Semantic::Tangent)
                .map_err(TypedStorageBuildError::Control)?
            {
                return Err(TypedStorageBuildError::Invariant);
            }
        }
    }
    if let Some(origins) = edit.origins {
        if origins.face_offsets.len() != p.face_count as usize + 1
            || origins.face_offsets.first() != Some(&0)
            || origins.face_offsets.last().copied().map(|n| n as usize)
                != Some(origins.triangles.len())
        {
            return Err(TypedStorageBuildError::Invariant);
        }
        let mut start = 0_u32;
        for face in 0..p.face_count as usize {
            let (first, last) = (
                origins.face_offsets[face] as usize,
                origins.face_offsets[face + 1] as usize,
            );
            let size = if p.topology == TopologyKind::Polygons {
                *p.polygon_boundaries
                    .and_then(|r| r.get(&source.u32_values))
                    .and_then(|s| s.get(face))
                    .ok_or(TypedStorageBuildError::Invariant)?
            } else {
                3
            };
            if size < 3 || last.checked_sub(first) != Some((size - 2) as usize) {
                return Err(TypedStorageBuildError::Invariant);
            }
            for triangle in origins
                .triangles
                .get(first..last)
                .ok_or(TypedStorageBuildError::Invariant)?
            {
                let valid = control
                    .run_unit(|| match p.topology {
                        TopologyKind::Polygons => {
                            triangle.iter().all(|&c| c >= start && c < start + size)
                                && triangle[0] != triangle[1]
                                && triangle[1] != triangle[2]
                                && triangle[2] != triangle[0]
                        }
                        TopologyKind::TriangleList => {
                            *triangle == [face as u32 * 3, face as u32 * 3 + 1, face as u32 * 3 + 2]
                        }
                        TopologyKind::TriangleStrip => {
                            let f = face as u32;
                            *triangle
                                == if face % 2 == 0 {
                                    [f, f + 1, f + 2]
                                } else {
                                    [f, f + 2, f + 1]
                                }
                        }
                        TopologyKind::TriangleFan => {
                            let f = face as u32;
                            *triangle == [f + 1, f + 2, 0]
                        }
                        _ => false,
                    })
                    .map_err(TypedStorageBuildError::Control)?;
                if !valid {
                    return Err(TypedStorageBuildError::Invariant);
                }
            }
            start = start.checked_add(size).ok_or_else(overflow)?;
        }
    } else if (edit.normals.is_some() || edit.tangents.is_some())
        && p.topology != TopologyKind::TriangleList
    {
        return Err(TypedStorageBuildError::Invariant);
    }
    let corners = match edit.origins {
        Some(o) => o.triangles.len().checked_mul(3).ok_or_else(overflow)?,
        None => p.corner_count as usize,
    };
    u32::try_from(corners).map_err(|_| overflow())?;
    for i in 0..edit.normals.map_or(0, VectorValues::len) {
        if !control
            .run_unit(|| {
                edit.normals.is_some_and(|n| {
                    n.len() == corners
                        && n.get(i).is_some_and(|v| {
                            v.iter().all(|v| v.is_finite())
                                && (v.iter().map(|v| v * v).sum::<f64>() - 1.0).abs()
                                    <= n.unit_length_tolerance()
                        })
                })
            })
            .map_err(TypedStorageBuildError::Control)?
        {
            return Err(TypedStorageBuildError::Invariant);
        }
    }
    for i in 0..edit.tangents.map_or(0, VectorValues::len) {
        if !control
            .run_unit(|| {
                edit.tangents.is_some_and(|n| {
                    n.len() == corners
                        && n.get(i).is_some_and(|v| {
                            v.iter().all(|v| v.is_finite())
                                && v[3].abs() == 1.0
                                && (v[..3].iter().map(|v| v * v).sum::<f64>() - 1.0).abs()
                                    <= n.unit_length_tolerance()
                        })
                })
            })
            .map_err(TypedStorageBuildError::Control)?
        {
            return Err(TypedStorageBuildError::Invariant);
        }
    }
    if edit.normals.is_some_and(|v| v.len() != corners)
        || edit.tangents.is_some_and(|v| v.len() != corners)
    {
        return Err(TypedStorageBuildError::Invariant);
    }
    Ok(())
}

enum MappingPass<'a> {
    Planning(&'a mut [LocalMapping]),
    Sealed(&'a [LocalMapping]),
}
impl MappingPass<'_> {
    fn get(&self, index: usize) -> Option<LocalMapping> {
        match self {
            Self::Planning(rows) => rows.get(index).copied(),
            Self::Sealed(rows) => rows.get(index).copied(),
        }
    }
    fn record(&mut self, index: usize, mapping: LocalMapping) {
        if let Self::Planning(rows) = self {
            rows[index] = mapping;
        }
    }
}

fn reconstruct<C: TypedStorageBuildControl>(
    source: &GeometryStorage,
    edits: &[SceneGeometryReplacement<'_>],
    mut mappings: MappingPass<'_>,
    w: &mut Writer<'_, C>,
) -> EditResult<(), C::Error> {
    let mut selected = 0;
    for (row, &primitive) in source.primitives.iter().enumerate() {
        let edit = edits
            .get(selected)
            .filter(|_| mappings.get(selected).is_some_and(|m| m.row == row));
        let start = w.counts[2];
        let source_range = primitive
            .attributes
            .checked(source.attributes.len())
            .ok_or(TypedStorageBuildError::Invariant)?;
        let mut mapping = edit.and_then(|_| mappings.get(selected)).map(|mut m| {
            m.position = None;
            m.normal = None;
            m.tangent = None;
            m.uv = None;
            m.smoothing = None;
            m.output_normal = None;
            m.output_tangent = None;
            m
        });
        if let Some(m) = &mut mapping {
            m.source_start = source_range.start;
            m.source_count = source_range.len();
            m.output_start = start;
        }
        let mut normal_pending = edit.and_then(|e| e.normals);
        let mut tangent_pending = edit.and_then(|e| e.tangents);
        for index in source_range {
            let mut attribute = source.attributes[index];
            if let (Some(e), Some(m)) = (edit, &mut mapping) {
                let duplicate = match attribute.semantic {
                    Semantic::Position => {
                        m.position = Some(index as u32);
                        false
                    }
                    Semantic::Normal
                        if e.normals.is_some() || attribute.domain == e.normal_domain =>
                    {
                        m.normal.replace(index as u32).is_some() && e.normals.is_some()
                    }
                    Semantic::Tangent => {
                        m.tangent.replace(index as u32).is_some()
                            && (e.tangents.is_some() || e.remove_tangents)
                    }
                    Semantic::SmoothingGroup => {
                        m.smoothing = Some(index as u32);
                        false
                    }
                    Semantic::Texcoord(set)
                        if set == e.uv_set && attribute.domain == e.uv_domain =>
                    {
                        m.uv = Some(index as u32);
                        false
                    }
                    _ => false,
                };
                if duplicate {
                    return Err(TypedStorageBuildError::Invariant);
                }
            }
            if !matches!(attribute.semantic, Semantic::Position) && normal_pending.is_some() {
                let index = generated_normal(normal_pending.take().unwrap(), w)?;
                if let Some(m) = &mut mapping {
                    m.output_normal = Some(index as u32);
                }
            }
            if !matches!(attribute.semantic, Semantic::Position | Semantic::Normal)
                && tangent_pending.is_some()
            {
                let index = generated_tangent(tangent_pending.take().unwrap(), w)?;
                if let Some(m) = &mut mapping {
                    m.output_tangent = Some(index as u32);
                }
            }
            if edit.is_some_and(|e| {
                e.normals.is_some() && attribute.semantic == Semantic::Normal
                    || (e.remove_tangents || e.tangents.is_some())
                        && attribute.semantic == Semantic::Tangent
            }) {
                continue;
            }
            let count = output_count(attribute, edit);
            attribute.payload = copy_payload(source, attribute, edit, count, w)?;
            attribute.element_count = count as u32;
            if let Semantic::Custom(name) = attribute.semantic {
                attribute.semantic = Semantic::Custom(w.name(source, name, NameKind::Custom)?);
            }
            if let Some(conventions) = &mut attribute.tangent_conventions {
                if let StoredTangentBasis::Namespaced { name, .. } = &mut conventions.basis {
                    *name = w.name(source, *name, NameKind::TangentBasis)?;
                }
                if let StoredTangentEncoding::Namespaced { name, .. } = &mut conventions.encoding {
                    *name = w.name(source, *name, NameKind::TangentEncoding)?;
                }
            }
            let output_index = w.attribute(attribute)? as u32;
            if let Some(m) = &mut mapping {
                match attribute.semantic {
                    Semantic::Normal if m.output_normal.is_none() => {
                        m.output_normal = Some(output_index)
                    }
                    Semantic::Tangent if m.output_tangent.is_none() => {
                        m.output_tangent = Some(output_index)
                    }
                    _ => {}
                }
            }
        }
        if let Some(values) = normal_pending {
            let index = generated_normal(values, w)?;
            if let Some(m) = &mut mapping {
                m.output_normal = Some(index as u32);
            }
        }
        if let Some(values) = tangent_pending {
            let index = generated_tangent(values, w)?;
            if let Some(m) = &mut mapping {
                m.output_tangent = Some(index as u32);
            }
        }
        if let (Some(_), Some(mut m)) = (edit, mapping) {
            m.output_count = w.counts[2] - start;
            mappings.record(selected, m);
            selected += 1;
        }
    }
    if selected != edits.len() {
        return Err(TypedStorageBuildError::Invariant);
    }
    selected = 0;
    let mut start = 0;
    for (row, &primitive) in source.primitives.iter().enumerate() {
        let actual = w.actual_primitive()?;
        let edit = edits
            .get(selected)
            .filter(|_| mappings.get(selected).is_some_and(|m| m.row == row));
        let count = edit
            .and_then(|_| mappings.get(selected))
            .map_or(primitive.attributes.len(), |m| m.output_count);
        let mut output = StoredPrimitive {
            attributes: TableRange::new(start, count),
            ..primitive
        };
        if let Some(origins) = edit.and_then(|e| e.origins) {
            let first = w.counts[7];
            w.project(7, actual.and_then(|p| p.indices));
            let indices = primitive
                .indices
                .map(|r| {
                    r.get(&source.u32_values)
                        .ok_or(TypedStorageBuildError::Invariant)
                })
                .transpose()?;
            for triangle in origins.triangles {
                for &corner in triangle {
                    w.u32(indices.map_or(Ok(corner), |s| {
                        s.get(corner as usize)
                            .copied()
                            .ok_or(TypedStorageBuildError::Invariant)
                    })?)?;
                }
            }
            output.topology = TopologyKind::TriangleList;
            output.corner_count = u32::try_from(w.counts[7] - first).map_err(|_| overflow())?;
            output.face_count = u32::try_from(origins.triangles.len()).map_err(|_| overflow())?;
            output.indices = Some(TableRange::new(first, w.counts[7] - first));
            output.polygon_boundaries = None;
        } else {
            let mut copy = |range: Option<TableRange<u32>>,
                            actual: Option<TableRange<u32>>|
             -> EditResult<Option<TableRange<u32>>, C::Error> {
                range
                    .map(|r| {
                        let start = w.counts[7];
                        w.project(7, actual);
                        for &value in r
                            .get(&source.u32_values)
                            .ok_or(TypedStorageBuildError::Invariant)?
                        {
                            w.u32(value)?;
                        }
                        Ok(TableRange::new(start, w.counts[7] - start))
                    })
                    .transpose()
            };
            output.polygon_boundaries = copy(
                primitive.polygon_boundaries,
                actual.and_then(|p| p.polygon_boundaries),
            )?;
            output.indices = copy(primitive.indices, actual.and_then(|p| p.indices))?;
        }
        w.primitive(output)?;
        start += count;
        if edit.is_some() {
            selected += 1;
        }
    }
    for &mesh in &source.logical_meshes {
        let name = mesh
            .name
            .map(|n| w.name(source, n, NameKind::Mesh))
            .transpose()?;
        w.mesh(StoredLogicalMesh { name, ..mesh })?;
    }
    for &instance in &source.mesh_instances {
        w.instance(instance)?;
    }
    Ok(())
}

fn output_count(a: StoredAttribute, edit: Option<&SceneGeometryReplacement<'_>>) -> usize {
    match (a.domain, edit.and_then(|e| e.origins)) {
        (AttributeDomain::Corner, Some(o)) => o.triangles.len() * 3,
        (AttributeDomain::Face, Some(o)) => o.triangles.len(),
        _ => a.element_count as usize,
    }
}

fn source_element<C: TypedStorageBuildControl>(
    a: StoredAttribute,
    edit: Option<&SceneGeometryReplacement<'_>>,
    destination: usize,
    face: &mut usize,
    w: &mut Writer<'_, C>,
) -> EditResult<usize, C::Error> {
    w.action(|| match (a.domain, edit.and_then(|e| e.origins)) {
        (AttributeDomain::Corner, Some(o)) => {
            o.triangles[destination / 3][destination % 3] as usize
        }
        (AttributeDomain::Face, Some(o)) => {
            while destination >= o.face_offsets[*face + 1] as usize {
                *face += 1;
            }
            *face
        }
        _ => destination,
    })
}

fn copy_payload<C: TypedStorageBuildControl>(
    source: &GeometryStorage,
    a: StoredAttribute,
    edit: Option<&SceneGeometryReplacement<'_>>,
    count: usize,
    w: &mut Writer<'_, C>,
) -> EditResult<Payload, C::Error> {
    let lanes = usize::from(a.lanes);
    let remapped = edit.is_some_and(|e| e.origins.is_some())
        && matches!(a.domain, AttributeDomain::Corner | AttributeDomain::Face);
    let actual = w.actual_payload()?;
    macro_rules! dense {
        ($range:expr, $pool:ident, $slot:expr, $emit:ident, $variant:ident) => {{
            let actual = match actual {
                Some(Payload::$variant(range)) => Some(range),
                None => None,
                _ => return Err(TypedStorageBuildError::Invariant),
            };
            w.project($slot, actual);
            let values = $range
                .get(&source.$pool)
                .ok_or(TypedStorageBuildError::Invariant)?;
            let start = w.counts[$slot];
            let mut face = 0;
            if remapped {
                for destination in 0..count {
                    let index = source_element(a, edit, destination, &mut face, w)?;
                    for &value in values
                        .get(index * lanes..index * lanes + lanes)
                        .ok_or(TypedStorageBuildError::Invariant)?
                    {
                        w.$emit(value)?;
                    }
                }
            } else {
                for &value in values {
                    w.$emit(value)?;
                }
            }
            Payload::$variant(TableRange::new(start, w.counts[$slot] - start))
        }};
    }
    macro_rules! sparse {
        ($indices:expr, $values:expr, $default:expr, $pool:ident, $slot:expr, $emit:ident, $variant:ident) => {{
            let (actual_indices, actual_values, actual_default) = match actual {
                Some(Payload::$variant {
                    indices,
                    values,
                    default,
                }) => (Some(indices), Some(values), Some(default)),
                None => (None, None, None),
                _ => return Err(TypedStorageBuildError::Invariant),
            };
            let indices = $indices
                .get(&source.u32_values)
                .ok_or(TypedStorageBuildError::Invariant)?;
            let values = $values
                .get(&source.$pool)
                .ok_or(TypedStorageBuildError::Invariant)?;
            let defaults = $default
                .get(&source.$pool)
                .ok_or(TypedStorageBuildError::Invariant)?;
            let start = w.counts[7];
            let mut face = 0;
            w.project(7, actual_indices);
            if remapped {
                for destination in 0..count {
                    let index = source_element(a, edit, destination, &mut face, w)?;
                    if lookup(indices, index, w)?.is_some() {
                        w.u32(destination as u32)?;
                    }
                }
            } else {
                for &index in indices {
                    w.u32(index)?;
                }
            }
            let output_indices = TableRange::new(start, w.counts[7] - start);
            let start = w.counts[$slot];
            let mut face = 0;
            w.project($slot, actual_values);
            if remapped {
                for destination in 0..count {
                    let index = source_element(a, edit, destination, &mut face, w)?;
                    if let Some(index) = lookup(indices, index, w)? {
                        for &value in values
                            .get(index * lanes..index * lanes + lanes)
                            .ok_or(TypedStorageBuildError::Invariant)?
                        {
                            w.$emit(value)?;
                        }
                    }
                }
            } else {
                for &value in values {
                    w.$emit(value)?;
                }
            }
            let output_values = TableRange::new(start, w.counts[$slot] - start);
            let start = w.counts[$slot];
            w.project($slot, actual_default);
            for &value in defaults {
                w.$emit(value)?;
            }
            Payload::$variant {
                indices: output_indices,
                values: output_values,
                default: TableRange::new(start, w.counts[$slot] - start),
            }
        }};
    }
    Ok(match a.payload {
        Payload::DenseF32(r) => dense!(r, f32_values, 5, f32, DenseF32),
        Payload::DenseF64(r) => dense!(r, f64_values, 6, f64, DenseF64),
        Payload::DenseU32(r) => dense!(r, u32_values, 7, u32, DenseU32),
        Payload::DenseI32(r) => dense!(r, i32_values, 8, i32, DenseI32),
        Payload::SparseF32 {
            indices,
            values,
            default,
        } => sparse!(indices, values, default, f32_values, 5, f32, SparseF32),
        Payload::SparseF64 {
            indices,
            values,
            default,
        } => sparse!(indices, values, default, f64_values, 6, f64, SparseF64),
        Payload::SparseU32 {
            indices,
            values,
            default,
        } => sparse!(indices, values, default, u32_values, 7, u32, SparseU32),
        Payload::SparseI32 {
            indices,
            values,
            default,
        } => sparse!(indices, values, default, i32_values, 8, i32, SparseI32),
    })
}

fn lookup<C: TypedStorageBuildControl>(
    indices: &[u32],
    index: usize,
    w: &mut Writer<'_, C>,
) -> EditResult<Option<usize>, C::Error> {
    let mut first = 0;
    let mut last = indices.len();
    while first < last {
        let middle = first + (last - first) / 2;
        match w.action(|| (indices[middle] as usize).cmp(&index))? {
            std::cmp::Ordering::Less => first = middle + 1,
            std::cmp::Ordering::Greater => last = middle,
            std::cmp::Ordering::Equal => return Ok(Some(middle)),
        }
    }
    Ok(None)
}

fn generated_normal<C: TypedStorageBuildControl>(
    values: VectorValues<'_, 3>,
    w: &mut Writer<'_, C>,
) -> EditResult<usize, C::Error> {
    let payload = generated(values, w)?;
    w.attribute(StoredAttribute {
        semantic: Semantic::Normal,
        tangent_conventions: None,
        domain: AttributeDomain::Corner,
        lanes: 3,
        element_count: values.len() as u32,
        payload,
    })
}
fn generated_tangent<C: TypedStorageBuildControl>(
    values: VectorValues<'_, 4>,
    w: &mut Writer<'_, C>,
) -> EditResult<usize, C::Error> {
    let payload = generated(values, w)?;
    w.attribute(StoredAttribute {
        semantic: Semantic::Tangent,
        tangent_conventions: Some(StoredTangentConventions {
            basis: StoredTangentBasis::TriangleUvDerivative,
            encoding: StoredTangentEncoding::SignInW,
        }),
        domain: AttributeDomain::Corner,
        lanes: 4,
        element_count: values.len() as u32,
        payload,
    })
}
fn generated<const N: usize, C: TypedStorageBuildControl>(
    values: VectorValues<'_, N>,
    w: &mut Writer<'_, C>,
) -> EditResult<Payload, C::Error> {
    let actual = w.actual_payload()?;
    Ok(match values {
        VectorValues::F32(values) => {
            let actual = match actual {
                Some(Payload::DenseF32(range)) => Some(range),
                None => None,
                _ => return Err(TypedStorageBuildError::Invariant),
            };
            w.project(5, actual);
            let first = w.counts[5];
            for &value in values.as_flattened() {
                w.f32(value)?;
            }
            Payload::DenseF32(TableRange::new(first, w.counts[5] - first))
        }
        VectorValues::F64(values) => {
            let actual = match actual {
                Some(Payload::DenseF64(range)) => Some(range),
                None => None,
                _ => return Err(TypedStorageBuildError::Invariant),
            };
            w.project(6, actual);
            let first = w.counts[6];
            for &value in values.as_flattened() {
                w.f64(value)?;
            }
            Payload::DenseF64(TableRange::new(first, w.counts[6] - first))
        }
    })
}

impl std::fmt::Debug for GeometrySelectionPlan<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeometrySelectionPlan")
            .field("targets", &self.edits.len())
            .field("work", &self.work)
            .finish_non_exhaustive()
    }
}
