#![forbid(unsafe_code)]

//! A concrete offline consumer using only public Baozi and native-candidate APIs.

mod buffers;

pub use buffers::*;

use std::{path::Path, time::Instant};

use baozi::{
    AssetIo, AssetPath, Axis, FrontFaceWinding, Handedness, ImportConfigOverlay, ImportDimension,
    ImportEvidence, ImportOperationControl, ImportRequest, ImportRequestLayer, Importer,
    OptionCompileLimits, PrimitiveRef, ResourceLimitsOverlay, TargetSpace, TopologyKind,
    ValidatedScene,
    postprocess::{
        NormalOptions, PostProcessControl, PostProcessReport, SpaceConversionOptions,
        convert_scene_space, generate_normals, triangulate_primitive,
    },
};
use baozi_native_io_candidate::{FileSystemAssetIo, NativeDirectoryCapability};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub target: TargetSpace,
    pub source_assumptions: Option<TargetSpace>,
    pub max_work_units: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            target: TargetSpace::try_new(
                Handedness::Left,
                Axis::PositiveY,
                Axis::NegativeZ,
                0.01,
                FrontFaceWinding::CounterClockwise,
            )
            .expect("fixed application coordinate convention is valid"),
            source_assumptions: None,
            max_work_units: 200_000_000,
        }
    }
}

#[derive(Debug, Default)]
pub struct Metrics {
    pub import_micros: u128,
    pub postprocess_micros: u128,
    pub buffers_micros: u128,
    pub import_peak_live_bytes: u64,
    pub import_work: u64,
    pub postprocess_work: u64,
    /// Maximum operation-local peak, not a sum or an aggregate physical-memory peak.
    pub max_postprocess_peak_live_bytes: u64,
    pub opened_assets: u64,
    pub changed_operations: usize,
}

#[derive(Debug)]
pub struct Output {
    pub asset: ApplicationAsset,
    pub metrics: Metrics,
    /// The original import evidence remains readable after Scene destruction.
    pub import_evidence: ImportEvidence,
}

pub fn process_file(root: &Path, relative: &str, options: Options) -> Result<Output> {
    let io = FileSystemAssetIo::from_capability(NativeDirectoryCapability::open_ambient(root)?);
    process(&io, AssetPath::new(relative)?, options)
}

pub fn process(io: &dyn AssetIo, source: AssetPath, options: Options) -> Result<Output> {
    let importer = Importer::new()?;
    let mut limits = ResourceLimitsOverlay::default();
    limits.max_work_units = Some(options.max_work_units);
    let request = importer.prepare_request(
        &ImportRequest::new().with_caller_layer(ImportRequestLayer::new(
            ImportConfigOverlay::default().with_resource_limits(limits),
        )),
        OptionCompileLimits::default(),
    )?;
    let started = Instant::now();
    let report = importer.read_asset_with_request(
        io,
        source,
        &request,
        ImportOperationControl::default(),
    )?;
    let mut metrics = Metrics {
        import_micros: started.elapsed().as_micros(),
        import_peak_live_bytes: report.ledger().peak_live_bytes(),
        import_work: report
            .ledger()
            .dimension(ImportDimension::WorkUnits)
            .map_or(0, |d| d.consumed()),
        opened_assets: report
            .ledger()
            .dimension(ImportDimension::AssetOpen)
            .map_or(0, |d| d.consumed()),
        ..Metrics::default()
    };
    let (mut scene, import_evidence) = report.into_parts();
    let started = Instant::now();
    // These operations preserve logical mesh/primitive order. Reacquire a current-generation
    // handle at each edit boundary; never reuse a collected ID after an earlier edit publishes.
    for mesh in 0..scene.mesh_count() {
        let count = scene
            .meshes()
            .nth(mesh)
            .ok_or("missing logical mesh")?
            .primitive_count();
        for primitive in 0..count {
            let current = primitive_at(&scene, mesh, primitive)?;
            let id = current.id();
            match current.topology() {
                TopologyKind::Polygons => {
                    let (next, report) =
                        triangulate_primitive(scene, id, PostProcessControl::default())?
                            .into_parts();
                    record(&mut metrics, &report);
                    scene = next;
                }
                TopologyKind::TriangleList => {}
                topology => return Err(format!("application requires triangle meshes, found {topology:?} in mesh {mesh}, primitive {primitive}").into()),
            }
        }
    }
    // Normal generation uses the stored numeric cross product. Establish CCW fronts before
    // generating a missing field, including when the caller declares a clockwise source.
    let normal_space = TargetSpace::try_new(
        options.target.handedness(),
        options.target.up_axis(),
        options.target.front_axis(),
        options.target.unit_scale_to_meters(),
        FrontFaceWinding::CounterClockwise,
    )?;
    let (next, report) = convert_scene_space(
        scene,
        SpaceConversionOptions {
            target: normal_space,
            source_assumptions: options.source_assumptions,
        },
        PostProcessControl::default(),
    )?
    .into_parts();
    record(&mut metrics, &report);
    drop(report);
    scene = next;
    for mesh in 0..scene.mesh_count() {
        let count = scene
            .meshes()
            .nth(mesh)
            .ok_or("missing logical mesh")?
            .primitive_count();
        for primitive in 0..count {
            let id = primitive_at(&scene, mesh, primitive)?.id();
            let (next, report) = generate_normals(
                scene,
                id,
                NormalOptions::default(),
                PostProcessControl::default(),
            )?
            .into_parts();
            record(&mut metrics, &report);
            scene = next;
        }
    }
    if options.target.front_face_winding() != normal_space.front_face_winding() {
        let (next, report) = convert_scene_space(
            scene,
            SpaceConversionOptions {
                target: options.target,
                source_assumptions: None,
            },
            PostProcessControl::default(),
        )?
        .into_parts();
        record(&mut metrics, &report);
        scene = next;
    }
    metrics.postprocess_micros = started.elapsed().as_micros();

    let started = Instant::now();
    let asset = ApplicationAsset::from_scene(&scene)?;
    metrics.buffers_micros = started.elapsed().as_micros();
    drop(scene);
    // Only application-owned buffers and detached import evidence escape this function.
    Ok(Output {
        asset,
        metrics,
        import_evidence,
    })
}

fn primitive_at(scene: &ValidatedScene, mesh: usize, primitive: usize) -> Result<PrimitiveRef<'_>> {
    scene
        .meshes()
        .nth(mesh)
        .and_then(|m| m.primitives().nth(primitive))
        .ok_or_else(|| format!("missing mesh {mesh}, primitive {primitive} after edit").into())
}

fn record(metrics: &mut Metrics, report: &PostProcessReport) {
    let resources = report.resources();
    metrics.postprocess_work += resources.consumed_work_items();
    metrics.max_postprocess_peak_live_bytes = metrics
        .max_postprocess_peak_live_bytes
        .max(resources.peak_live_bytes());
    metrics.changed_operations += usize::from(matches!(
        report.disposition(),
        baozi::postprocess::PostProcessRunDisposition::Changed
    ));
}
