//! Fixed geometry-only comparison endpoints; the full asset workflow is independent.

use std::{collections::HashMap, fmt, path::Path, time::Instant};

use baozi::{
    AssetPath, AttributeData, AttributeDomain, AttributeElement, AttributeSemanticRef,
    ImportConfigOverlay, ImportOperationControl, ImportRequest, ImportRequestLayer, Importer,
    OptionCompileLimits, PrimitiveRef, ResourceLimitsOverlay, SpatialQueryLimits, TopologyKind,
    ValidatedScene,
    postprocess::{
        NormalOptions, PostProcessControl, TriangleLowering, TrianglePreparationOptions,
        list_vertices, prepare_triangles,
    },
};
use baozi_native_io_candidate::{FileSystemAssetIo, NativeDirectoryCapability};
use serde_json::{Value, json};

use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Import,
    Prepare,
    Extract,
    Pipeline,
}

impl Stage {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "import" => Ok(Self::Import),
            "prepare" => Ok(Self::Prepare),
            "extract" => Ok(Self::Extract),
            "pipeline" => Ok(Self::Pipeline),
            _ => Err("unknown comparison stage".into()),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Import => "import",
            Self::Prepare => "prepare",
            Self::Extract => "extract",
            Self::Pipeline => "pipeline",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoMode {
    File,
    Memory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fields {
    Position,
    PositionNormal,
    PositionNormalUv0,
}

impl Fields {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Position => "position",
            Self::PositionNormal => "position-normal",
            Self::PositionNormalUv0 => "position-normal-uv0",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub stage: Stage,
    pub io_mode: IoMode,
    pub fields: Fields,
    pub generate_missing_normals: bool,
    pub max_work_units: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            stage: Stage::Import,
            io_mode: IoMode::Memory,
            fields: Fields::Position,
            generate_missing_normals: false,
            max_work_units: 200_000_000,
        }
    }
}

/// Owned selected arrays, without materials, indexing, deformation or canonical identities.
#[derive(Debug)]
pub struct Draw {
    pub topology: TopologyKind,
    pub source_scalar_bytes: usize,
    pub positions: Vec<[f64; 3]>,
    pub normals: Option<Vec<[f64; 3]>>,
    pub uv0: Option<Vec<[f64; 2]>>,
}

#[derive(Debug)]
pub struct Placement {
    pub draw: usize,
    pub world: [[f64; 4]; 4],
}

#[derive(Debug)]
pub struct Output {
    pub draws: Vec<Draw>,
    pub placements: Vec<Placement>,
}

#[derive(Debug)]
pub struct Measurement {
    pub summary: Value,
    pub output: Option<Output>,
}

#[derive(Debug)]
struct Unsupported(&'static str);

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for Unsupported {}

#[derive(Debug)]
struct Failure {
    phase: &'static str,
    source: Box<dyn std::error::Error>,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.phase, self.source)
    }
}

impl std::error::Error for Failure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

/// Executes one endpoint with exact native-object lifetimes.
pub fn measure_file(root: &Path, source: &str, options: Options) -> Result<Measurement> {
    let mut phase = "setup";
    measure_inner(root, source, options, &mut phase)
        .map_err(|source| Box::new(Failure { phase, source }) as Box<dyn std::error::Error>)
}

fn measure_inner(
    root: &Path,
    source: &str,
    options: Options,
    phase: &mut &'static str,
) -> Result<Measurement> {
    let source_path = AssetPath::new(source)?;
    // Memory input is symmetric harness state, prepared before every endpoint clock.
    let preload = if options.io_mode == IoMode::Memory {
        Some(std::fs::read(root.join(source_path.as_str()))?)
    } else {
        None
    };
    let pipeline_started = Instant::now();
    let io = if options.io_mode == IoMode::File {
        Some(FileSystemAssetIo::from_capability(
            NativeDirectoryCapability::open_ambient(root)?,
        ))
    } else {
        None
    };
    let importer = Importer::new()?;
    let mut limits = ResourceLimitsOverlay::default();
    limits.max_work_units = Some(options.max_work_units);
    let request = importer.plan(
        &ImportRequest::new().with_caller_layer(ImportRequestLayer::new(
            ImportConfigOverlay::default().with_resource_limits(limits),
        )),
        OptionCompileLimits::default(),
    )?;
    *phase = "import";
    let import_started = Instant::now();
    let report = if let Some(bytes) = &preload {
        request
            .read_bytes(
                AssetPath::new(source)?,
                bytes,
                ImportOperationControl::default(),
            )
            .map_err(baozi::Error::from)?
    } else {
        request
            .read_asset(
                io.as_ref().ok_or("missing file IO")?,
                source_path,
                ImportOperationControl::default(),
            )
            .map_err(baozi::Error::from)?
    };
    let import_ns = import_started.elapsed().as_nanos();
    let (mut scene, evidence) = report.into_parts();
    let import_work = evidence
        .ledger()
        .dimension(baozi::ImportDimension::WorkUnits)
        .map_or(0, |entry| entry.consumed());
    *phase = "observe";
    let scalar_bytes = if options.stage == Stage::Pipeline {
        None
    } else {
        Some(source_width(&scene)?)
    };
    let mut prepare_ns = 0;
    if options.stage != Stage::Import {
        *phase = "prepare";
        let started = Instant::now();
        if scene
            .meshes()
            .flat_map(|mesh| mesh.primitives())
            .any(|primitive| primitive.topology() != TopologyKind::Points)
        {
            let targets: Vec<_> = scene
                .meshes()
                .flat_map(|mesh| mesh.primitives())
                .map(|primitive| primitive.id())
                .collect();
            let (prepared, report) = prepare_triangles(
                scene,
                &targets,
                TrianglePreparationOptions {
                    lowering: TriangleLowering::Preserve,
                    normals: options
                        .generate_missing_normals
                        .then(NormalOptions::default),
                    ..TrianglePreparationOptions::default()
                },
                PostProcessControl::default(),
            )?
            .into_parts();
            scene = prepared;
            drop(report);
            drop(targets);
        } else if options.stage == Stage::Prepare {
            return Err(Unsupported("point cases have no preparation endpoint").into());
        }
        prepare_ns = started.elapsed().as_nanos();
    }
    let mut extract_ns = 0;
    let output = if matches!(options.stage, Stage::Extract | Stage::Pipeline) {
        *phase = "extract";
        let started = Instant::now();
        let output = extract(&scene, options.fields)?;
        extract_ns = started.elapsed().as_nanos();
        Some(output)
    } else {
        None
    };
    *phase = "observe";
    let (observation, output_bytes, stage_ns, scalar_bytes) = if options.stage == Stage::Pipeline {
        drop((scene, evidence, request, importer, io));
        let stage_ns = pipeline_started.elapsed().as_nanos();
        let output = output
            .as_ref()
            .ok_or("pipeline must retain selected output")?;
        (
            observe_output(output)?,
            Some(output.bytes()),
            stage_ns,
            uniform_width(output.draws.iter().map(|draw| Ok(draw.source_scalar_bytes)))?,
        )
    } else {
        let observation = if let Some(output) = &output {
            observe_output(output)?
        } else {
            observe_scene(&scene, options.fields, options.stage == Stage::Import)?
        };
        let bytes = output.as_ref().map(Output::bytes);
        drop((scene, evidence, request, importer, io));
        let stage_ns = match options.stage {
            Stage::Import => import_ns,
            Stage::Prepare => prepare_ns,
            Stage::Extract => extract_ns,
            Stage::Pipeline => unreachable!("handled above"),
        };
        (
            observation,
            bytes,
            stage_ns,
            scalar_bytes.ok_or("missing source scalar observation")?,
        )
    };
    // Keep the preloaded source alive through exactly the same endpoints as the C++ worker.
    std::hint::black_box(&preload);
    Ok(Measurement {
        summary: json!({
            "schema": "baozi-assimp-observation/v1", "engine": "baozi", "status": "success",
            "stage": options.stage.name(), "io_mode": if options.io_mode == IoMode::File { "file" } else { "memory" },
            "fields": options.fields.name(), "normal_policy": if options.generate_missing_normals { "missing" } else { "preserve" },
            "scalar_bytes": scalar_bytes, "computation_scalar_bytes": null,
            "observation_scalar_bytes": 8, "output_scalar_bytes": 8,
            "computation_note": "Observation, copy and base-pose math use F64; native import/preparation have no single global arithmetic width",
            "stage_ns": stage_ns, "observation": observation.json(),
            "output": output_bytes.map(|(logical, capacity)| json!({"logical_bytes": logical, "capacity_bytes": capacity})),
            "uv_convention": "source", "import_work": import_work,
            "import_budget_enforced": true,
        }),
        output,
    })
}

pub fn failure(error: &(dyn std::error::Error + 'static), options: Options) -> Value {
    let (phase, cause) = error
        .downcast_ref::<Failure>()
        .map_or(("setup", error), |failure| {
            (failure.phase, failure.source.as_ref())
        });
    let status = if cause.is::<Unsupported>() {
        "unsupported"
    } else if let Some(baozi::Error::Import(failure)) = cause.downcast_ref::<baozi::Error>() {
        if matches!(failure.cause(), baozi::ImportCause::UnsupportedFormat) {
            "unsupported"
        } else {
            "policy_rejected"
        }
    } else {
        "policy_rejected"
    };
    json!({"schema": "baozi-assimp-observation/v1", "engine": "baozi", "status": status,
        "stage": options.stage.name(), "io_mode": if options.io_mode == IoMode::File { "file" } else { "memory" },
        "fields": options.fields.name(), "normal_policy": if options.generate_missing_normals { "missing" } else { "preserve" },
        "stage_ns": null, "observation": null, "output": null, "failure_stage": phase, "diagnostic": error.to_string()})
}

fn source_width(scene: &ValidatedScene) -> Result<usize> {
    uniform_width(
        scene
            .meshes()
            .flat_map(|mesh| mesh.primitives())
            .map(position_width),
    )
}

fn position_width(primitive: PrimitiveRef<'_>) -> Result<usize> {
    let position = primitive
        .attribute(AttributeSemanticRef::Position, AttributeDomain::Vertex)
        .ok_or(Unsupported("missing vertex positions"))?;
    match position.data() {
        AttributeData::DenseF32 { .. } | AttributeData::SparseF32 { .. } => Ok(4),
        AttributeData::DenseF64 { .. } | AttributeData::SparseF64 { .. } => Ok(8),
        _ => Err(Unsupported("nonfloating positions").into()),
    }
}

fn uniform_width(widths: impl IntoIterator<Item = Result<usize>>) -> Result<usize> {
    let mut width = None;
    for current in widths {
        let current = current?;
        if width.is_some_and(|width| width != current) {
            return Err(Unsupported("mixed source widths need a distinct comparison lane").into());
        }
        width = Some(current);
    }
    width.ok_or_else(|| Unsupported("empty geometry has no scalar lane").into())
}

fn domain(
    primitive: PrimitiveRef<'_>,
    semantic: AttributeSemanticRef<'_>,
) -> Result<Option<AttributeDomain>> {
    let mut domains = primitive
        .attributes()
        .filter(|attribute| attribute.semantic() == semantic)
        .map(|attribute| attribute.domain());
    let domain = domains.next();
    if domains.next().is_some() {
        return Err(Unsupported("ambiguous simultaneous field domains").into());
    }
    Ok(domain)
}

fn numeric<const N: usize>(element: AttributeElement<'_>) -> Result<[f64; N]> {
    super::buffers::numeric(element)
}

fn visit_draw(
    primitive: PrimitiveRef<'_>,
    fields: Fields,
    allow_missing: bool,
    mut visit: impl FnMut([f64; 3], Option<[f64; 3]>, Option<[f64; 2]>) -> Result<()>,
) -> Result<()> {
    if !matches!(
        primitive.topology(),
        TopologyKind::TriangleList | TopologyKind::Points
    ) {
        return Err(Unsupported("comparison admits only triangle lists and points").into());
    }
    let normal = if fields != Fields::Position {
        domain(primitive, AttributeSemanticRef::Normal)?
    } else {
        None
    };
    let uv = if fields == Fields::PositionNormalUv0 {
        domain(primitive, AttributeSemanticRef::Texcoord(0))?
    } else {
        None
    };
    if !allow_missing
        && ((fields != Fields::Position && normal.is_none())
            || (fields == Fields::PositionNormalUv0 && uv.is_none()))
    {
        return Err(Unsupported("requested field is missing").into());
    }
    for tuple in list_vertices(primitive)?.vertices() {
        let position = numeric(
            tuple
                .attribute(AttributeSemanticRef::Position, AttributeDomain::Vertex)
                .ok_or(Unsupported("position tuple is missing"))?
                .element(),
        )?;
        let normal = normal
            .map(|domain| {
                numeric(
                    tuple
                        .attribute(AttributeSemanticRef::Normal, domain)
                        .ok_or(Unsupported("normal tuple is missing"))?
                        .element(),
                )
            })
            .transpose()?;
        let uv = uv
            .map(|domain| {
                numeric(
                    tuple
                        .attribute(AttributeSemanticRef::Texcoord(0), domain)
                        .ok_or(Unsupported("UV tuple is missing"))?
                        .element(),
                )
            })
            .transpose()?;
        visit(position, normal, uv)?;
    }
    Ok(())
}

fn extract(scene: &ValidatedScene, fields: Fields) -> Result<Output> {
    let mut draws = Vec::new();
    draws.try_reserve_exact(scene.primitive_count())?;
    for primitive in scene.meshes().flat_map(|mesh| mesh.primitives()) {
        let count = list_vertices(primitive)?.len();
        let mut draw = Draw {
            topology: primitive.topology(),
            source_scalar_bytes: position_width(primitive)?,
            positions: reserved(count)?,
            normals: (fields != Fields::Position)
                .then(|| reserved(count))
                .transpose()?,
            uv0: (fields == Fields::PositionNormalUv0)
                .then(|| reserved(count))
                .transpose()?,
        };
        visit_draw(primitive, fields, false, |position, normal, uv| {
            draw.positions.push(position);
            if let Some(values) = &mut draw.normals {
                values.push(normal.ok_or("missing selected normal")?);
            }
            if let Some(values) = &mut draw.uv0 {
                values.push(uv.ok_or("missing selected UV")?);
            }
            Ok(())
        })?;
        draws.push(draw);
    }
    let transforms =
        scene.base_pose_world_transforms(SpatialQueryLimits::new(200_000_000, 256 << 20), None)?;
    let mut placements = Vec::new();
    let placement_count = scene
        .mesh_instances()
        .map(|instance| instance.mesh().primitive_count())
        .sum();
    placements.try_reserve_exact(placement_count)?;
    let mut first_draw = HashMap::new();
    first_draw.try_reserve(scene.mesh_count())?;
    let mut offset = 0;
    for mesh in scene.meshes() {
        first_draw.insert(mesh.id(), offset);
        offset += mesh.primitive_count();
    }
    for instance in scene.mesh_instances() {
        let world = transforms.node_transform(instance.node().id())?;
        let first = first_draw[&instance.mesh().id()];
        for draw in first..first + instance.mesh().primitive_count() {
            placements.push(Placement { draw, world });
        }
    }
    Ok(Output { draws, placements })
}

fn reserved<T>(count: usize) -> Result<Vec<T>> {
    let mut values = Vec::new();
    values.try_reserve_exact(count)?;
    Ok(values)
}

#[derive(Default)]
struct Bounds<const N: usize>(Option<[[f64; N]; 2]>);

impl<const N: usize> Bounds<N> {
    fn add(&mut self, value: [f64; N]) {
        let bounds = self.0.get_or_insert([value, value]);
        for (lane, value) in value.into_iter().enumerate() {
            bounds[0][lane] = bounds[0][lane].min(value);
            bounds[1][lane] = bounds[1][lane].max(value);
        }
    }
}

#[derive(Default)]
struct Observation {
    draws: usize,
    placements: usize,
    tuples: usize,
    bounds: Bounds<3>,
    normal_bounds: Bounds<3>,
    uv0_bounds: Bounds<2>,
    area: f64,
    translation_x_sum: f64,
    triangle: [[f64; 3]; 3],
}

impl Observation {
    fn vertex(
        &mut self,
        ordinal: usize,
        triangles: bool,
        position: [f64; 3],
        normal: Option<[f64; 3]>,
        uv: Option<[f64; 2]>,
    ) {
        self.tuples += 1;
        self.bounds.add(position);
        if let Some(normal) = normal {
            self.normal_bounds.add(normal);
        }
        if let Some(uv) = uv {
            self.uv0_bounds.add(uv);
        }
        if triangles {
            self.triangle[ordinal % 3] = position;
            if ordinal % 3 == 2 {
                let [a, b, c] = self.triangle;
                let u = std::array::from_fn::<_, 3, _>(|i| b[i] - a[i]);
                let v = std::array::from_fn::<_, 3, _>(|i| c[i] - a[i]);
                let cross = [
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ];
                self.area += cross[0].hypot(cross[1]).hypot(cross[2]) * 0.5;
            }
        }
    }

    fn json(&self) -> Value {
        json!({"draws": self.draws, "placements": self.placements, "tuples": self.tuples,
            "bounds": self.bounds.0, "normal_bounds": self.normal_bounds.0, "uv0_bounds": self.uv0_bounds.0,
            "area": self.area, "translation_x_sum": self.translation_x_sum})
    }
}

fn observe_scene(
    scene: &ValidatedScene,
    fields: Fields,
    allow_missing: bool,
) -> Result<Observation> {
    let mut observation = Observation::default();
    for primitive in scene.meshes().flat_map(|mesh| mesh.primitives()) {
        observation.draws += 1;
        let mut ordinal = 0;
        visit_draw(primitive, fields, allow_missing, |position, normal, uv| {
            observation.vertex(
                ordinal,
                primitive.topology() == TopologyKind::TriangleList,
                position,
                normal,
                uv,
            );
            ordinal += 1;
            Ok(())
        })?;
    }
    let transforms =
        scene.base_pose_world_transforms(SpatialQueryLimits::new(200_000_000, 256 << 20), None)?;
    for instance in scene.mesh_instances() {
        let count = instance.mesh().primitive_count();
        observation.placements += count;
        observation.translation_x_sum +=
            transforms.node_transform(instance.node().id())?[3][0] * count as f64;
    }
    Ok(observation)
}

fn observe_output(output: &Output) -> Result<Observation> {
    let mut observation = Observation {
        draws: output.draws.len(),
        placements: output.placements.len(),
        ..Observation::default()
    };
    for draw in &output.draws {
        for (ordinal, &position) in draw.positions.iter().enumerate() {
            observation.vertex(
                ordinal,
                draw.topology == TopologyKind::TriangleList,
                position,
                draw.normals.as_ref().map(|values| values[ordinal]),
                draw.uv0.as_ref().map(|values| values[ordinal]),
            );
        }
    }
    for placement in &output.placements {
        if placement.draw >= output.draws.len() {
            return Err("invalid detached draw reference".into());
        }
        observation.translation_x_sum += placement.world[3][0];
    }
    Ok(observation)
}

impl Output {
    fn bytes(&self) -> (usize, usize) {
        let mut logical = self.placements.len() * (size_of::<[[f64; 4]; 4]>() + size_of::<usize>());
        let mut capacity = self.placements.capacity() * size_of::<Placement>()
            + self.draws.capacity() * size_of::<Draw>();
        for draw in &self.draws {
            logical += draw.positions.len() * size_of::<[f64; 3]>();
            capacity += draw.positions.capacity() * size_of::<[f64; 3]>();
            if let Some(values) = &draw.normals {
                logical += values.len() * size_of::<[f64; 3]>();
                capacity += values.capacity() * size_of::<[f64; 3]>();
            }
            if let Some(values) = &draw.uv0 {
                logical += values.len() * size_of::<[f64; 2]>();
                capacity += values.capacity() * size_of::<[f64; 2]>();
            }
        }
        (logical, capacity)
    }
}
