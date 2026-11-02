//! Pure CPU planning and checked narrowing. Source tuple ordinals are never welded.

use crate::deformation::{DrawFrames, frames_for_draw};
use crate::material::CompiledMaterial;
use crate::{Control, Error, Result, add, cap, f32_at, mul, vec_with_capacity};
use baozi::{MaterialAssignment, TopologyKind};
use baozi_asset_consumer::{ApplicationAsset, DrawBuffer, EvaluatedAsset, MorphStream};
use std::{mem::size_of, ops::Range};

pub const VERTEX_STRIDE: u64 = 64;
pub const SOURCE_NORMAL: u32 = 1;
pub const SOURCE_TANGENT: u32 = 2;
pub const SOURCE_COLOR: u32 = 4;
pub const WORLD_TANGENT: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarError {
    NonFinite,
    Overflow,
    Underflow,
    SubnormalUnsupported,
    PrecisionExceeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScalarLocation {
    pub instance: usize,
    pub draw: usize,
    pub vertex: usize,
    pub stream: &'static str,
    pub lane: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct Tolerance {
    pub absolute: f64,
    pub relative: f64,
}

impl Tolerance {
    pub const POSITION: Self = Self {
        absolute: 1e-6,
        relative: 1e-6,
    };
    pub const ATTRIBUTE: Self = Self {
        absolute: 1.0 / 1_048_576.0,
        relative: 1.0 / 1_048_576.0,
    };
    fn validate(self) -> Result<()> {
        if [self.absolute, self.relative]
            .iter()
            .any(|v| !v.is_finite() || *v < 0.0)
        {
            Err(Error::InvalidLimit(
                "scalar tolerances must be finite and nonnegative",
            ))
        } else {
            Ok(())
        }
    }
}

pub fn narrow(x: f64, tolerance: Tolerance, location: ScalarLocation) -> Result<f32> {
    tolerance.validate()?;
    let failure = |kind| Error::Scalar { location, kind };
    if !x.is_finite() {
        return Err(failure(ScalarError::NonFinite));
    }
    let y = x as f32;
    if !y.is_finite() {
        return Err(failure(ScalarError::Overflow));
    }
    if x != 0.0 && y == 0.0 {
        return Err(failure(ScalarError::Underflow));
    }
    if y.is_subnormal() {
        return Err(failure(ScalarError::SubnormalUnsupported));
    }
    let allowed = tolerance.absolute + tolerance.relative * x.abs();
    if !allowed.is_finite() {
        return Err(Error::InvalidLimit("scalar error envelope overflow"));
    }
    if (f64::from(y) - x).abs() > allowed {
        return Err(failure(ScalarError::PrecisionExceeded));
    }
    Ok(y)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexWidth {
    U16,
    U32,
}
impl IndexWidth {
    pub fn bytes(self) -> u64 {
        match self {
            Self::U16 => 2,
            Self::U32 => 4,
        }
    }
    pub fn vertices(self) -> u64 {
        match self {
            Self::U16 => 65_536,
            Self::U32 => u64::from(u32::MAX) + 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PalettePolicy {
    /// All rows remain on CPU; the existing evaluator uses the complete instance skin.
    CpuFullRows,
    /// Validate deterministic palette remaps; this does not enable GPU skinning.
    Bounded {
        max_joints: u32,
        allow_cpu_fallback: bool,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct DeviceLimits {
    pub index_width: IndexWidth,
    pub max_vertices_per_chunk: u64,
    pub max_indices_per_chunk: u32,
    pub max_chunk_bytes: u64,
    pub max_output_vertices: u64,
    pub max_output_indices: u64,
    pub max_chunks: u64,
    pub max_expansion: u32,
    pub max_plan_bytes: u64,
    pub max_gpu_bytes: u64,
    pub max_work: u64,
    pub max_cpu_joints: u32,
    pub palette: PalettePolicy,
    pub position_tolerance: Tolerance,
    pub attribute_tolerance: Tolerance,
}
impl Default for DeviceLimits {
    fn default() -> Self {
        Self {
            index_width: IndexWidth::U16,
            max_vertices_per_chunk: 65_536,
            max_indices_per_chunk: 1_048_575,
            max_chunk_bytes: 128 * 1024 * 1024,
            max_output_vertices: 2_000_000,
            max_output_indices: 8_000_000,
            max_chunks: 4096,
            max_expansion: 4,
            max_plan_bytes: 512 * 1024 * 1024,
            max_gpu_bytes: 512 * 1024 * 1024,
            max_work: 200_000_000,
            max_cpu_joints: 4096,
            palette: PalettePolicy::CpuFullRows,
            position_tolerance: Tolerance::POSITION,
            attribute_tolerance: Tolerance::ATTRIBUTE,
        }
    }
}

#[derive(Debug)]
pub struct InfluenceRows {
    pub offsets: Vec<usize>,
    /// Local palette slots, including zero weights and duplicate joint entries.
    pub joints: Vec<u32>,
    pub weights: Vec<f32>,
}

#[derive(Debug)]
pub struct Chunk {
    pub instance: usize,
    pub draw: usize,
    pub elements: Range<usize>,
    pub material: MaterialAssignment<usize>,
    pub compiled_material: usize,
    pub vertex_sources: Vec<u32>,
    pub indices: Vec<u32>,
    pub vertices: Vec<u8>,
    /// Vertex-major, all source UV sets in the source's ascending set order.
    pub uv_bytes: Vec<u8>,
    pub uv_sets: Vec<u32>,
    pub active_uv_ordinals: [u32; 4],
    /// Source Normal/Tangent/Color presence, plus WORLD_TANGENT when the packed
    /// tangent is evaluated world data. Otherwise a present tangent is source data.
    pub field_flags: u32,
    pub orientation: i8,
    pub palette_policy: PalettePolicy,
    pub palette_source_joints: Vec<u32>,
    pub influences: Option<InfluenceRows>,
}

impl Chunk {
    pub fn index_bytes(&self, width: IndexWidth) -> Result<Vec<u8>> {
        let count = usize::try_from(mul(self.indices.len() as u64, width.bytes())?)
            .map_err(|_| Error::InvalidLimit("index byte count"))?;
        let mut out = vec_with_capacity(count)?;
        for &index in &self.indices {
            match width {
                IndexWidth::U16 => out.extend_from_slice(
                    &u16::try_from(index)
                        .map_err(|_| Error::InvalidAsset("U16 local index"))?
                        .to_le_bytes(),
                ),
                IndexWidth::U32 => out.extend_from_slice(&index.to_le_bytes()),
            }
        }
        Ok(out)
    }
}

/// Private borrowed source/evaluation prevent substituting an input during upload.
/// Maps also cover retained, inactive streams; unused vertices remain in `source_draw`.
pub struct DevicePlan<'a> {
    asset: &'a ApplicationAsset,
    evaluation: &'a EvaluatedAsset,
    chunks: Vec<Chunk>,
    materials: Vec<CompiledMaterial>,
    limits: DeviceLimits,
    plan_bytes: u64,
    gpu_bytes: u64,
    work: u64,
}

// Counts owned Vec capacities, including simultaneous scratch and relocation overlap.
// Input borrows, stack records and allocator metadata are outside this byte contract.
pub(crate) struct Planning<'a> {
    limits: DeviceLimits,
    control: Control<'a>,
    work: u64,
    live_bytes: u64,
    #[cfg(test)]
    cancel_at_work: Option<u64>,
    #[cfg(test)]
    reservations: usize,
}

impl<'a> Planning<'a> {
    fn new(limits: DeviceLimits, control: Control<'a>) -> Self {
        Self {
            limits,
            control,
            work: 0,
            live_bytes: 0,
            #[cfg(test)]
            cancel_at_work: None,
            #[cfg(test)]
            reservations: 0,
        }
    }
    pub(crate) fn charge(&mut self, amount: u64) -> Result<()> {
        #[cfg(test)]
        if self.cancel_at_work.is_some_and(|at| self.work >= at)
            && let Some(flag) = self.control.cancellation
        {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.control.check()?;
        let next = add(self.work, amount)?;
        cap("planning work", next, self.limits.max_work)?;
        self.work = next;
        Ok(())
    }
    fn admit(&self, additional: u64) -> Result<()> {
        cap(
            "plan bytes",
            add(self.live_bytes, additional)?,
            self.limits.max_plan_bytes,
        )
    }
    fn retain(&mut self, bytes: u64) -> Result<()> {
        self.admit(bytes)?;
        self.live_bytes = add(self.live_bytes, bytes)?;
        Ok(())
    }
    fn release(&mut self, bytes: u64) {
        self.live_bytes -= bytes;
    }
    fn reserve<T>(&mut self, values: &mut Vec<T>, capacity: usize) -> Result<()> {
        self.charge(0)?;
        if capacity <= values.capacity() {
            return Ok(());
        }
        let old = mul(values.capacity() as u64, size_of::<T>() as u64)?;
        let requested = mul(capacity as u64, size_of::<T>() as u64)?;
        // The old allocation remains live until a relocating reservation completes.
        self.admit(requested)?;
        self.charge(mul(values.len() as u64, size_of::<T>() as u64)?)?;
        values.try_reserve_exact(capacity - values.len())?;
        #[cfg(test)]
        {
            self.reservations += 1;
        }
        let actual = mul(values.capacity() as u64, size_of::<T>() as u64)?;
        self.admit(actual)?;
        self.live_bytes = add(self.live_bytes - old, actual)?;
        self.charge(0)
    }
    fn grow<T>(&mut self, values: &mut Vec<T>, needed: usize, maximum: u64) -> Result<()> {
        if needed <= values.capacity() {
            return Ok(());
        }
        let capacity = add(values.capacity().max(1) as u64, values.capacity() as u64)?
            .max(needed as u64)
            .min(maximum);
        cap("vector capacity", needed as u64, capacity)?;
        self.reserve(
            values,
            usize::try_from(capacity).map_err(|_| Error::InvalidLimit("vector capacity"))?,
        )
    }
    fn zeros(&mut self, count: usize) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.reserve(&mut out, count)?;
        while out.len() < count {
            let next = out.len() + 256.min(count - out.len());
            self.charge((next - out.len()) as u64)?;
            out.resize(next, 0);
        }
        Ok(out)
    }
}

impl<'a> DevicePlan<'a> {
    pub fn asset(&self) -> &'a ApplicationAsset {
        self.asset
    }
    pub fn evaluation(&self) -> &'a EvaluatedAsset {
        self.evaluation
    }
    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }
    pub fn materials(&self) -> &[CompiledMaterial] {
        &self.materials
    }
    pub fn limits(&self) -> DeviceLimits {
        self.limits
    }
    pub fn plan_bytes(&self) -> u64 {
        self.plan_bytes
    }
    pub fn gpu_bytes(&self) -> u64 {
        self.gpu_bytes
    }
    pub fn work(&self) -> u64 {
        self.work
    }
    pub fn source_draw(&self, chunk: usize) -> Option<&'a DrawBuffer> {
        self.chunks
            .get(chunk)
            .and_then(|c| self.asset.draws.get(c.draw))
    }
    pub fn source_morph(&self, chunk: usize, stream: usize) -> Option<(&'a MorphStream, &[u32])> {
        let c = self.chunks.get(chunk)?;
        Some((
            self.asset.draws.get(c.draw)?.morphs.get(stream)?,
            &c.vertex_sources,
        ))
    }
    pub fn source_influence_row(
        &self,
        chunk: usize,
        local: usize,
    ) -> Option<(&'a [u32], &'a [f32])> {
        let c = self.chunks.get(chunk)?;
        let vertex = *c.vertex_sources.get(local)? as usize;
        let rows = self.asset.draws.get(c.draw)?.influences.as_ref()?;
        let range = *rows.offsets.get(vertex)?..*rows.offsets.get(vertex + 1)?;
        Some((rows.joints.get(range.clone())?, rows.weights.get(range)?))
    }

    pub fn prepare(
        asset: &'a ApplicationAsset,
        evaluation: &'a EvaluatedAsset,
        frames: &[DrawFrames<'_>],
        limits: DeviceLimits,
        control: Control<'_>,
    ) -> Result<Self> {
        control.check()?;
        limits.position_tolerance.validate()?;
        limits.attribute_tolerance.validate()?;
        if limits.max_vertices_per_chunk == 0
            || limits.max_indices_per_chunk < 3
            || limits.max_expansion == 0
        {
            return Err(Error::InvalidLimit(
                "nonzero triangle/output limits required",
            ));
        }
        let mut planning = Planning::new(limits, control);
        let mut out = Self {
            asset,
            evaluation,
            chunks: Vec::new(),
            materials: Vec::new(),
            limits,
            plan_bytes: 0,
            gpu_bytes: 0,
            work: 0,
        };
        // Validate each source table once, independent of the number of chunks or placements.
        for (draw, source) in asset.draws.iter().enumerate() {
            planning.charge(1)?;
            validate_draw(source, draw, asset.mesh_count, &mut planning)?;
        }
        for f in frames {
            planning.charge(1)?;
            let placement = asset
                .instances
                .get(f.instance)
                .ok_or(Error::InvalidAsset("explicit frame instance"))?;
            let source = asset
                .draws
                .get(f.draw)
                .ok_or(Error::InvalidAsset("explicit frame draw"))?;
            if source.mesh != placement.mesh {
                return Err(Error::InvalidAsset("explicit frame placement"));
            }
        }
        let mut expected = 0usize;
        let mut source_vertices = 0u64;
        let mut output_vertices = 0u64;
        let mut output_indices = 0u64;
        for (instance, placement) in asset.instances.iter().enumerate() {
            planning.charge(1)?;
            if placement.mesh >= asset.mesh_count || placement.node >= asset.nodes.len() {
                return Err(Error::InvalidAsset("instance reference"));
            }
            let skin = placement
                .skin
                .map(|skin| {
                    asset
                        .skins
                        .get(skin)
                        .ok_or(Error::InvalidAsset("skin index"))
                })
                .transpose()?;
            if let Some(skin) = skin {
                cap(
                    "full CPU joint matrices",
                    skin.joints.len() as u64,
                    u64::from(limits.max_cpu_joints),
                )?;
                if skin.inverse_binds.len() != skin.joints.len() {
                    return Err(Error::InvalidAsset("skin inverse-bind correspondence"));
                }
                for &node in &skin.joints {
                    planning.charge(1)?;
                    if node >= asset.nodes.len() {
                        return Err(Error::InvalidAsset("skin joint node"));
                    }
                }
            }
            for (draw, source) in asset.draws.iter().enumerate() {
                planning.charge(1)?;
                if source.mesh != placement.mesh {
                    continue;
                }
                let evaluated = evaluation
                    .draws
                    .get(expected)
                    .ok_or(Error::InvalidAsset("missing evaluated draw"))?;
                expected += 1;
                if evaluated.instance != instance
                    || evaluated.draw != draw
                    || evaluated.world_positions.len() != source.vertices.len()
                {
                    return Err(Error::InvalidAsset("evaluation correspondence"));
                }
                // This is a current-table reference check, even for unused vertices and zero weights.
                // Retained unbound rows deliberately have no instance palette bound.
                if let Some(skin) = skin {
                    let rows = source
                        .influences
                        .as_ref()
                        .ok_or(Error::InvalidAsset("skinned draw has no influences"))?;
                    for &joint in &rows.joints {
                        planning.charge(1)?;
                        if joint as usize >= skin.joints.len() {
                            return Err(Error::InvalidAsset(
                                "influence joint outside instance skin",
                            ));
                        }
                    }
                }
                let source_material = source.material.effective();
                let mut compiled = None;
                for (i, material) in out.materials.iter().enumerate() {
                    planning.charge(1)?;
                    if material.source == source_material {
                        compiled = Some(i);
                        break;
                    }
                }
                let compiled = if let Some(i) = compiled {
                    i
                } else {
                    cap(
                        "compiled materials",
                        add(out.materials.len() as u64, 1)?,
                        limits.max_chunks,
                    )?;
                    let count = out.materials.len() + 1;
                    planning.grow(&mut out.materials, count, limits.max_chunks)?;
                    let source = source_material
                        .map(|i| {
                            asset
                                .materials
                                .get(i)
                                .ok_or(Error::InvalidAsset("material index"))
                        })
                        .transpose()?;
                    // The fixed compiler reserves SLOT_COUNT UV ordinals internally.
                    let uv_capacity = mul(crate::material::SLOT_COUNT as u64, 4)?;
                    planning.admit(uv_capacity)?;
                    planning.charge(add(
                        256,
                        mul(source.map_or(0, |m| m.bindings.len()) as u64, 256)?,
                    )?)?;
                    let material = CompiledMaterial::compile(asset, source_material)?;
                    planning.retain(mul(material.active_uv_sets.capacity() as u64, 4)?)?;
                    planning.charge(0)?;
                    out.materials.push(material);
                    out.materials.len() - 1
                };
                let material = &out.materials[compiled];
                for &set in &material.active_uv_sets {
                    uv_ordinal(source, draw, set, &mut planning)?;
                }
                let weights = evaluation
                    .morph_weights
                    .get(instance)
                    .ok_or(Error::InvalidAsset("morph weight table"))?;
                for morph in &source.morphs {
                    planning.charge(1)?;
                    let weight = *weights
                        .get(morph.target)
                        .ok_or(Error::InvalidAsset("morph target index"))?;
                    if material.normal_maps().next().is_some()
                        && morph.semantic == baozi::MorphDeltaSemantic::Tangent
                        && weight != 0.0
                    {
                        for values in morph.values.chunks(256) {
                            planning.charge(mul(values.len() as u64, 3)?)?;
                            if values.iter().flatten().any(|&v| v != 0.0) {
                                return Err(Error::UnsupportedTangentMorphNormalMap { draw });
                            }
                        }
                    }
                }
                let world_frames =
                    frames_for_draw(source, evaluated, frames, !material.unlit, &mut planning)?;
                let flags = source.vertices.first().map_or(0, vertex_flags)
                    | if world_frames.has_tangents() {
                        WORLD_TANGENT
                    } else {
                        0
                    };
                // One dense lookup per draw has an exact, observable Vec capacity. Only the
                // chunk's used entries are reset; forced splits never clear the whole table.
                let mut lookup = Vec::<usize>::new();
                planning.reserve(&mut lookup, source.vertices.len())?;
                while lookup.len() < source.vertices.len() {
                    let next = lookup.len() + 256.min(source.vertices.len() - lookup.len());
                    planning.charge((next - lookup.len()) as u64)?;
                    lookup.resize(next, usize::MAX);
                }
                for indices in source.indices.chunks(256) {
                    planning.charge(indices.len() as u64)?;
                    for &v in indices {
                        if lookup[v as usize] == usize::MAX {
                            source_vertices = add(source_vertices, 1)?;
                            lookup[v as usize] = 0;
                        }
                    }
                }
                for indices in source.indices.chunks(256) {
                    planning.charge(indices.len() as u64)?;
                    for &v in indices {
                        lookup[v as usize] = usize::MAX;
                    }
                }
                let mut cursor = 0usize;
                while cursor < source.indices.len() / 3 {
                    planning.charge(1)?;
                    cap(
                        "chunks",
                        add(out.chunks.len() as u64, 1)?,
                        limits.max_chunks,
                    )?;
                    let count = out.chunks.len() + 1;
                    planning.grow(&mut out.chunks, count, limits.max_chunks)?;
                    let mut chunk = Chunk {
                        instance,
                        draw,
                        elements: cursor..cursor,
                        material: source.material,
                        compiled_material: compiled,
                        vertex_sources: Vec::new(),
                        indices: Vec::new(),
                        vertices: Vec::new(),
                        uv_bytes: Vec::new(),
                        uv_sets: Vec::new(),
                        active_uv_ordinals: [u32::MAX; 4],
                        field_flags: flags,
                        orientation: 0,
                        palette_policy: limits.palette,
                        palette_source_joints: Vec::new(),
                        influences: None,
                    };
                    let remaining = source.indices.len() - cursor * 3;
                    let max_indices = remaining.min(limits.max_indices_per_chunk as usize) as u64;
                    let max_vertices = (source.vertices.len() as u64)
                        .min(max_indices)
                        .min(limits.max_vertices_per_chunk)
                        .min(limits.index_width.vertices());
                    let mut row_count = 0u64;
                    while cursor < source.indices.len() / 3 {
                        planning.charge(32)?;
                        let element = &source.indices[cursor * 3..cursor * 3 + 3];
                        let parity = world_frames.at(element[0] as usize).orientation;
                        if element
                            .iter()
                            .any(|&v| world_frames.at(v as usize).orientation != parity)
                        {
                            return Err(Error::MixedFrameOrientation {
                                instance,
                                draw,
                                element: cursor,
                            });
                        }
                        if chunk.orientation != 0 && chunk.orientation != parity {
                            break;
                        }
                        let mut new = [0; 3];
                        let mut new_count = 0usize;
                        for &v in element {
                            if lookup[v as usize] == usize::MAX && !new[..new_count].contains(&v) {
                                new[new_count] = v;
                                new_count += 1;
                            }
                        }
                        let vertices = add(chunk.vertex_sources.len() as u64, new_count as u64)?;
                        let indices = add(chunk.indices.len() as u64, 3)?;
                        let bytes = add(
                            mul(
                                vertices,
                                add(VERTEX_STRIDE, mul(source.uv_streams.len() as u64, 8)?)?,
                            )?,
                            mul(indices, limits.index_width.bytes())?,
                        )?;
                        let exceeded = if vertices
                            > limits
                                .max_vertices_per_chunk
                                .min(limits.index_width.vertices())
                        {
                            Some("local vertices")
                        } else if indices > u64::from(limits.max_indices_per_chunk) {
                            Some("local indices")
                        } else if bytes > limits.max_chunk_bytes {
                            Some("chunk bytes")
                        } else {
                            None
                        };
                        if let Some(dimension) = exceeded {
                            if chunk.indices.is_empty() {
                                return Err(Error::IndivisibleElementTooLarge {
                                    draw,
                                    element: cursor,
                                    dimension,
                                });
                            }
                            break;
                        }
                        cap(
                            "output vertices",
                            add(output_vertices, vertices)?,
                            limits.max_output_vertices,
                        )?;
                        cap(
                            "output indices",
                            add(output_indices, indices)?,
                            limits.max_output_indices,
                        )?;
                        let palette_start = chunk.palette_source_joints.len();
                        let mut added_rows = 0u64;
                        let mut palette_exceeded = false;
                        if let Some(rows) = &source.influences {
                            'rows: for &v in &new[..new_count] {
                                let range = rows.offsets[v as usize]..rows.offsets[v as usize + 1];
                                added_rows = add(added_rows, range.len() as u64)?;
                                for &joint in &rows.joints[range] {
                                    planning.charge(1)?;
                                    if joint_slot(
                                        &chunk.palette_source_joints,
                                        joint,
                                        &mut planning,
                                    )?
                                    .is_none()
                                    {
                                        let count = chunk.palette_source_joints.len() + 1;
                                        cap(
                                            "palette scratch",
                                            count as u64,
                                            u64::from(limits.max_cpu_joints),
                                        )?;
                                        if let PalettePolicy::Bounded {
                                            max_joints,
                                            allow_cpu_fallback,
                                        } = chunk.palette_policy
                                            && count > max_joints as usize
                                        {
                                            if chunk.indices.is_empty() && allow_cpu_fallback {
                                                chunk.palette_policy = PalettePolicy::CpuFullRows;
                                            } else {
                                                palette_exceeded = true;
                                                break 'rows;
                                            }
                                        }
                                        planning.grow(
                                            &mut chunk.palette_source_joints,
                                            count,
                                            u64::from(limits.max_cpu_joints),
                                        )?;
                                        chunk.palette_source_joints.push(joint);
                                    }
                                }
                            }
                        }
                        if palette_exceeded {
                            chunk.palette_source_joints.truncate(palette_start);
                            if chunk.indices.is_empty() {
                                return Err(Error::IndivisibleElementTooLarge {
                                    draw,
                                    element: cursor,
                                    dimension: "joint palette",
                                });
                            }
                            break;
                        }
                        // Grow only for admitted triangles; other limits can seal far before the draw tail.
                        planning.grow(
                            &mut chunk.vertex_sources,
                            vertices as usize,
                            max_vertices,
                        )?;
                        planning.grow(&mut chunk.indices, indices as usize, max_indices)?;
                        for &v in &new[..new_count] {
                            lookup[v as usize] = chunk.vertex_sources.len();
                            chunk.vertex_sources.push(v);
                        }
                        for &v in element {
                            chunk.indices.push(
                                u32::try_from(lookup[v as usize])
                                    .map_err(|_| Error::InvalidLimit("local vertex index"))?,
                            );
                        }
                        chunk.orientation = parity;
                        row_count = add(row_count, added_rows)?;
                        cursor += 1;
                    }
                    chunk.elements.end = cursor;
                    output_vertices = add(output_vertices, chunk.vertex_sources.len() as u64)?;
                    output_indices = add(output_indices, chunk.indices.len() as u64)?;
                    let vertex_bytes = mul(chunk.vertex_sources.len() as u64, VERTEX_STRIDE)?;
                    let uv_bytes = mul(
                        mul(
                            chunk.vertex_sources.len() as u64,
                            source.uv_streams.len() as u64,
                        )?,
                        8,
                    )?;
                    let index_bytes = add(
                        mul(chunk.indices.len() as u64, limits.index_width.bytes())?,
                        3,
                    )? & !3;
                    out.gpu_bytes = add(
                        out.gpu_bytes,
                        add(add(vertex_bytes, uv_bytes.max(8))?, add(index_bytes, 32)?)?,
                    )?;
                    cap("GPU geometry bytes", out.gpu_bytes, limits.max_gpu_bytes)?;
                    chunk.vertices = planning.zeros(
                        usize::try_from(vertex_bytes)
                            .map_err(|_| Error::InvalidLimit("vertex bytes"))?,
                    )?;
                    chunk.uv_bytes = planning.zeros(
                        usize::try_from(uv_bytes).map_err(|_| Error::InvalidLimit("UV bytes"))?,
                    )?;
                    if source.influences.is_some() {
                        let mut rows = InfluenceRows {
                            offsets: Vec::new(),
                            joints: Vec::new(),
                            weights: Vec::new(),
                        };
                        planning.reserve(&mut rows.offsets, chunk.vertex_sources.len() + 1)?;
                        let count = usize::try_from(row_count)
                            .map_err(|_| Error::InvalidLimit("influence row entries"))?;
                        planning.reserve(&mut rows.joints, count)?;
                        planning.reserve(&mut rows.weights, count)?;
                        rows.offsets.push(0);
                        chunk.influences = Some(rows);
                    }
                    for (local, &v) in chunk.vertex_sources.iter().enumerate() {
                        planning.charge(16)?;
                        let vertex = v as usize;
                        let location = |stream, lane| ScalarLocation {
                            instance,
                            draw,
                            vertex,
                            stream,
                            lane,
                        };
                        let f = world_frames.at(vertex);
                        let normal = f.normal;
                        // Normals-only evaluation preserves the authored tangent as SOURCE data.
                        let tangent = f
                            .tangent
                            .or(source.vertices[vertex].tangent)
                            .unwrap_or([1.0, 0.0, 0.0, 1.0]);
                        let color = source.vertices[vertex].color.unwrap_or([1.0; 4]);
                        if !(0.0..=1.0).contains(&color[3]) {
                            return Err(Error::InvalidAsset("vertex alpha outside unit interval"));
                        }
                        for (lane, value) in
                            evaluated.world_positions[vertex].into_iter().enumerate()
                        {
                            f32_at(
                                &mut chunk.vertices,
                                local * 64 + lane * 4,
                                narrow(
                                    value,
                                    limits.position_tolerance,
                                    location("position", lane),
                                )?,
                            );
                        }
                        for (offset, stream, values) in [
                            (16, "normal", [normal[0], normal[1], normal[2], 0.0]),
                            (32, "tangent", tangent),
                            (48, "color", color),
                        ] {
                            for (lane, value) in values.into_iter().enumerate() {
                                f32_at(
                                    &mut chunk.vertices,
                                    local * 64 + offset + lane * 4,
                                    narrow(
                                        value,
                                        limits.attribute_tolerance,
                                        location(stream, lane),
                                    )?,
                                );
                            }
                        }
                        for (set, stream) in source.uv_streams.iter().enumerate() {
                            planning.charge(2)?;
                            for (lane, value) in stream.values[vertex].into_iter().enumerate() {
                                f32_at(
                                    &mut chunk.uv_bytes,
                                    (local * source.uv_streams.len() + set) * 8 + lane * 4,
                                    narrow(
                                        value,
                                        limits.attribute_tolerance,
                                        location("uv", lane),
                                    )?,
                                );
                            }
                        }
                        if let (Some(source_rows), Some(rows)) =
                            (&source.influences, &mut chunk.influences)
                        {
                            pack_row(
                                source_rows,
                                source_rows.offsets[vertex]..source_rows.offsets[vertex + 1],
                                &chunk.palette_source_joints,
                                rows,
                                &mut planning,
                            )?;
                        }
                        lookup[vertex] = usize::MAX;
                    }
                    for (slot, &set) in material.active_uv_sets.iter().enumerate() {
                        chunk.active_uv_ordinals[slot] =
                            u32::try_from(uv_ordinal(source, draw, set, &mut planning)?)
                                .map_err(|_| Error::InvalidLimit("UV ordinal"))?;
                    }
                    planning.reserve(&mut chunk.uv_sets, source.uv_streams.len())?;
                    for stream in &source.uv_streams {
                        planning.charge(1)?;
                        chunk.uv_sets.push(stream.set);
                    }
                    check_normal_map_triangles(source, &chunk, material, &mut planning)?;
                    out.chunks.push(chunk);
                }
                planning.release(mul(lookup.capacity() as u64, size_of::<usize>() as u64)?);
                drop(lookup);
            }
        }
        if expected != evaluation.draws.len() {
            return Err(Error::InvalidAsset("extra evaluated draws"));
        }
        cap(
            "vertex expansion",
            output_vertices,
            mul(source_vertices, u64::from(limits.max_expansion))?,
        )?;
        planning.charge(0)?;
        out.plan_bytes = planning.live_bytes;
        out.work = planning.work;
        Ok(out)
    }
}

fn vertex_flags(v: &baozi_asset_consumer::Vertex) -> u32 {
    (u32::from(v.normal.is_some()) * SOURCE_NORMAL)
        | (u32::from(v.tangent.is_some()) * SOURCE_TANGENT)
        | (u32::from(v.color.is_some()) * SOURCE_COLOR)
}

fn validate_draw(
    source: &DrawBuffer,
    draw: usize,
    mesh_count: usize,
    planning: &mut Planning<'_>,
) -> Result<()> {
    if source.mesh >= mesh_count {
        return Err(Error::InvalidAsset("draw mesh index"));
    }
    if source.topology != TopologyKind::TriangleList {
        return Err(Error::UnsupportedTopology { draw });
    }
    if !source.indices.len().is_multiple_of(3) {
        return Err(Error::InvalidAsset("triangle indices"));
    }
    let flags = source.vertices.first().map_or(0, vertex_flags);
    for vertices in source.vertices.chunks(256) {
        planning.charge(mul(vertices.len() as u64, 16)?)?;
        if vertices.iter().any(|v| vertex_flags(v) != flags) {
            return Err(Error::InvalidAsset("mixed optional vertex-field presence"));
        }
    }
    for indices in source.indices.chunks(256) {
        planning.charge(indices.len() as u64)?;
        if indices.iter().any(|&v| v as usize >= source.vertices.len()) {
            return Err(Error::InvalidAsset("triangle indices"));
        }
    }
    let mut previous = None;
    for stream in &source.uv_streams {
        planning.charge(1)?;
        if previous.is_some_and(|set| set >= stream.set)
            || stream.values.len() != source.vertices.len()
        {
            return Err(Error::InvalidAsset("UV stream correspondence"));
        }
        previous = Some(stream.set);
    }
    for morph in &source.morphs {
        planning.charge(1)?;
        if morph.values.len() != source.vertices.len() {
            return Err(Error::InvalidAsset("morph stream correspondence"));
        }
    }
    if let Some(rows) = &source.influences {
        if rows.offsets.len().checked_sub(1) != Some(source.vertices.len())
            || rows.offsets.first() != Some(&0)
            || rows.offsets.last() != Some(&rows.joints.len())
            || rows.joints.len() != rows.weights.len()
        {
            return Err(Error::InvalidAsset("complete influence rows"));
        }
        for offsets in rows.offsets.windows(2) {
            planning.charge(1)?;
            if offsets[0] > offsets[1] {
                return Err(Error::InvalidAsset("complete influence rows"));
            }
        }
        for weights in rows.weights.chunks(256) {
            planning.charge(weights.len() as u64)?;
            if weights.iter().any(|w| !w.is_finite()) {
                return Err(Error::InvalidAsset("complete influence rows"));
            }
        }
    }
    Ok(())
}

fn uv_ordinal(
    source: &DrawBuffer,
    draw: usize,
    set: u32,
    planning: &mut Planning<'_>,
) -> Result<usize> {
    planning.charge(u64::from(source.uv_streams.len().checked_ilog2().unwrap_or(0)) + 1)?;
    source
        .uv_streams
        .binary_search_by_key(&set, |s| s.set)
        .map_err(|_| Error::MissingUv { draw, set })
}

fn joint_slot(palette: &[u32], joint: u32, planning: &mut Planning<'_>) -> Result<Option<usize>> {
    for (block, values) in palette.chunks(256).enumerate() {
        planning.charge(values.len() as u64)?;
        if let Some(slot) = values.iter().position(|&j| j == joint) {
            return Ok(Some(block * 256 + slot));
        }
    }
    Ok(None)
}

fn pack_row(
    source: &baozi_asset_consumer::Influences,
    range: Range<usize>,
    palette: &[u32],
    rows: &mut InfluenceRows,
    planning: &mut Planning<'_>,
) -> Result<()> {
    for entry in range {
        planning.charge(1)?;
        let slot = joint_slot(palette, source.joints[entry], planning)?
            .ok_or(Error::InvalidAsset("palette correspondence"))?;
        rows.joints
            .push(u32::try_from(slot).map_err(|_| Error::InvalidLimit("palette slot"))?);
        rows.weights.push(source.weights[entry]);
    }
    rows.offsets.push(rows.joints.len());
    Ok(())
}

fn check_normal_map_triangles(
    source: &DrawBuffer,
    c: &Chunk,
    material: &CompiledMaterial,
    planning: &mut Planning<'_>,
) -> Result<()> {
    for map in material.normal_maps() {
        let ordinal = uv_ordinal(source, c.draw, map.uv_set, planning)?;
        for (element, triangle) in c.indices.chunks_exact(3).enumerate() {
            planning.charge(64)?;
            let mut p = [[0.0f32; 3]; 3];
            let mut uv = [[0.0f32; 2]; 3];
            for i in 0..3 {
                let v = triangle[i] as usize;
                for (lane, value) in p[i].iter_mut().enumerate() {
                    let offset = v * 64 + lane * 4;
                    *value = f32::from_le_bytes(
                        c.vertices[offset..offset + 4]
                            .try_into()
                            .map_err(|_| Error::InvalidAsset("packed position"))?,
                    );
                }
                let mut raw = [0.0; 2];
                for (lane, value) in raw.iter_mut().enumerate() {
                    let offset = (v * source.uv_streams.len() + ordinal) * 8 + lane * 4;
                    *value = f32::from_le_bytes(
                        c.uv_bytes[offset..offset + 4]
                            .try_into()
                            .map_err(|_| Error::InvalidAsset("packed UV"))?,
                    );
                }
                uv[i] = map.coordinate(raw);
            }
            let a = std::array::from_fn(|i| f64::from(p[1][i] - p[0][i]));
            let b = std::array::from_fn(|i| f64::from(p[2][i] - p[0][i]));
            let cross = crate::deformation::cross(a, b);
            let d = (uv[1][0] - uv[0][0]) * (uv[2][1] - uv[0][1])
                - (uv[1][1] - uv[0][1]) * (uv[2][0] - uv[0][0]);
            if !d.is_finite() || d.abs() < 1e-20 || crate::deformation::dot(cross, cross) < 1e-30 {
                return Err(Error::DegenerateNormalMapFrame {
                    draw: c.draw,
                    element: c.elements.start + element,
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use baozi_asset_consumer::{EvaluatedDraw, Influences, Vertex};
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn palette_preflight_rejects_4096_entries_without_a_reservation() {
        let mut planning = Planning::new(
            DeviceLimits {
                max_plan_bytes: 1024,
                ..Default::default()
            },
            Control::default(),
        );
        let mut palette = Vec::<u32>::new();
        assert!(matches!(
            planning.reserve(&mut palette, 4096),
            Err(Error::Limit {
                dimension: "plan bytes",
                requested: 16_384,
                limit: 1024
            })
        ));
        assert_eq!(planning.reservations, 0);
        assert_eq!(palette.capacity(), 0);
    }

    #[test]
    fn growth_admits_resident_scratch_and_old_new_capacity_together() {
        let mut planning = Planning::new(
            DeviceLimits {
                max_plan_bytes: 96,
                ..Default::default()
            },
            Control::default(),
        );
        let mut resident = Vec::<u8>::new();
        let mut scratch = Vec::<u8>::new();
        planning.reserve(&mut resident, 32).unwrap();
        resident.resize(32, 0);
        planning.reserve(&mut scratch, 32).unwrap();
        scratch.resize(32, 0);
        assert_eq!(planning.live_bytes, 64);
        assert!(matches!(
            planning.reserve(&mut scratch, 64),
            Err(Error::Limit {
                dimension: "plan bytes",
                requested: 128,
                limit: 96
            })
        ));
        assert_eq!(planning.reservations, 2);
        assert_eq!(scratch.capacity(), 32);
    }

    #[test]
    fn row_packing_polls_and_charges_inside_a_single_long_vertex_row() {
        let source = Influences {
            offsets: vec![0, 1024],
            joints: vec![0; 1024],
            weights: vec![-0.0; 1024],
        };
        let flag = AtomicBool::new(false);
        let mut planning = Planning::new(
            DeviceLimits::default(),
            Control {
                cancellation: Some(&flag),
            },
        );
        let mut rows = InfluenceRows {
            offsets: Vec::new(),
            joints: Vec::new(),
            weights: Vec::new(),
        };
        planning.reserve(&mut rows.offsets, 2).unwrap();
        rows.offsets.push(0);
        planning.reserve(&mut rows.joints, 1024).unwrap();
        planning.reserve(&mut rows.weights, 1024).unwrap();
        planning.cancel_at_work = Some(64);
        assert!(matches!(
            pack_row(&source, 0..1024, &[0], &mut rows, &mut planning),
            Err(Error::Cancelled)
        ));
        assert!(flag.load(Ordering::Relaxed));
        assert_eq!(rows.joints.len(), 32);
        assert_eq!(rows.offsets, [0]);

        let mut planning = Planning::new(
            DeviceLimits {
                max_work: 64,
                ..Default::default()
            },
            Control::default(),
        );
        rows.joints.clear();
        rows.weights.clear();
        assert!(matches!(
            pack_row(&source, 0..1024, &[0], &mut rows, &mut planning),
            Err(Error::Limit {
                dimension: "planning work",
                requested: 65,
                limit: 64
            })
        ));
        assert_eq!(rows.joints.len(), 32);
    }

    #[test]
    fn palette_search_polls_between_blocks_before_finishing_one_entry() {
        let source = Influences {
            offsets: vec![0, 1],
            joints: vec![4095],
            weights: vec![1.0],
        };
        let palette: Vec<u32> = (0..4096).collect();
        let flag = AtomicBool::new(false);
        let mut planning = Planning::new(
            DeviceLimits::default(),
            Control {
                cancellation: Some(&flag),
            },
        );
        let mut rows = InfluenceRows {
            offsets: Vec::new(),
            joints: Vec::new(),
            weights: Vec::new(),
        };
        planning.reserve(&mut rows.offsets, 2).unwrap();
        rows.offsets.push(0);
        planning.reserve(&mut rows.joints, 1).unwrap();
        planning.reserve(&mut rows.weights, 1).unwrap();
        planning.cancel_at_work = Some(257);
        assert!(matches!(
            pack_row(&source, 0..1, &palette, &mut rows, &mut planning),
            Err(Error::Cancelled)
        ));
        assert_eq!(planning.work, 257);
        assert!(flag.load(Ordering::Relaxed));
        assert!(rows.joints.is_empty());
    }

    #[test]
    fn frame_validation_cancels_mid_table_without_allocating_frame_scratch() {
        let source = DrawBuffer {
            mesh: 0,
            primitive: 0,
            material: MaterialAssignment::Authored(0),
            topology: TopologyKind::TriangleList,
            vertices: vec![
                Vertex {
                    position: [0.0; 3],
                    normal: Some([0.0, 0.0, 1.0]),
                    tangent: None,
                    color: None
                };
                1024
            ],
            indices: vec![],
            uv_streams: vec![],
            tangent_conventions: None,
            influences: None,
            morphs: vec![],
        };
        let evaluated = EvaluatedDraw {
            instance: 0,
            draw: 0,
            positions: vec![[0.0; 3]; 1024],
            world_positions: vec![[0.0; 3]; 1024],
            world_normals: Some(vec![[0.0, 0.0, 1.0]; 1024]),
            world_tangents: None,
            world_orientations: Some(vec![1; 1024]),
        };
        let flag = AtomicBool::new(false);
        let mut planning = Planning::new(
            DeviceLimits::default(),
            Control {
                cancellation: Some(&flag),
            },
        );
        planning.cancel_at_work = Some(32);
        assert!(matches!(
            frames_for_draw(&source, &evaluated, &[], false, &mut planning),
            Err(Error::Cancelled)
        ));
        assert_eq!(planning.work, 32);
        assert!(flag.load(Ordering::Relaxed));
        assert_eq!(planning.reservations, 0);
    }
}
