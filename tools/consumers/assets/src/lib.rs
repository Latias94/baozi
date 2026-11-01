#![forbid(unsafe_code)]

//! A concrete offline consumer using only public Baozi and native-candidate APIs.

mod buffers;
mod evaluation;

pub use buffers::*;
pub use evaluation::*;

use std::{collections::HashSet, path::Path, time::Instant};

use baozi::{
    AssetIo, AssetPath, Axis, FrontFaceWinding, Handedness, ImportConfigOverlay, ImportDimension,
    ImportEvidence, ImportOperationControl, ImportRequest, ImportRequestLayer, Importer,
    OptionCompileLimits, ProvenanceKindRef, ResourceLimitsOverlay, TargetSpace, ValidatedScene,
    postprocess::{
        NormalOptions, NormalOrientation, PostProcessBudgets, PostProcessControl,
        PostProcessReport, SpaceConversionOptions, TrianglePreparationOptions, convert_scene_space,
        prepare_triangles,
    },
};
use baozi_native_io_candidate::{FileSystemAssetIo, NativeDirectoryCapability};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub target: TargetSpace,
    pub source_assumptions: Option<TargetSpace>,
    pub max_work_units: u64,
    pub postprocess_budgets: PostProcessBudgets,
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
            postprocess_budgets: PostProcessBudgets::default(),
        }
    }
}

#[derive(Debug, Default)]
pub struct Metrics {
    pub import_micros: u128,
    pub postprocess_micros: u128,
    /// Application traversal of retained owner/source relations, separate from library ledgers.
    pub source_trace_micros: u128,
    pub buffers_micros: u128,
    pub import_peak_live_bytes: u64,
    pub import_work: u64,
    pub postprocess_work: u64,
    /// Maximum operation-local peak, not a sum or an aggregate physical-memory peak.
    pub max_postprocess_peak_live_bytes: u64,
    pub opened_assets: u64,
    pub changed_operations: usize,
    pub selected_geometry_reconstructions: u32,
    pub traced_primitive_owners: usize,
    pub traced_provenance_nodes: usize,
    pub traced_source_objects: usize,
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
    let mut targets = Vec::new();
    targets.try_reserve_exact(scene.primitive_count())?;
    for mesh in scene.meshes() {
        for primitive in mesh.primitives() {
            targets.push(primitive.id());
        }
    }
    let (prepared, report) = prepare_triangles(
        scene,
        &targets,
        TrianglePreparationOptions {
            normals: Some(NormalOptions::default()),
            orientation: NormalOrientation::SceneFrontFace {
                assume_unknown: options
                    .source_assumptions
                    .map(TargetSpace::front_face_winding),
            },
            budgets: options.postprocess_budgets,
            ..TrianglePreparationOptions::default()
        },
        PostProcessControl::default(),
    )?
    .into_parts();
    record(&mut metrics, &report);
    metrics.selected_geometry_reconstructions = report
        .geometry_preparation()
        .map_or(0, |report| report.geometry_reconstruction_count());
    drop(report);
    drop(targets);
    let (converted, report) = convert_scene_space(
        prepared,
        SpaceConversionOptions {
            target: options.target,
            source_assumptions: options.source_assumptions,
        },
        PostProcessControl::default(),
    )?
    .into_parts();
    record(&mut metrics, &report);
    drop(report);
    scene = converted;
    metrics.postprocess_micros = started.elapsed().as_micros();

    let started = Instant::now();
    trace_primitive_sources(&scene, &mut metrics)?;
    metrics.source_trace_micros = started.elapsed().as_micros();

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

fn trace_primitive_sources(scene: &ValidatedScene, metrics: &mut Metrics) -> Result<()> {
    let mut pending = Vec::new();
    let mut visited = HashSet::new();
    let mut sources = HashSet::new();
    for primitive in scene.meshes().flat_map(|mesh| mesh.primitives()) {
        let attachments = scene.owner_provenance(primitive.metadata_owner())?;
        pending.try_reserve(attachments.len())?;
        pending.extend(attachments);
        metrics.traced_primitive_owners += 1;
    }
    // One shared-ancestor visit across all draws; no recursive walk or second Scene graph.
    while let Some(node) = pending.pop() {
        if visited.contains(&node.id()) {
            continue;
        }
        visited.try_reserve(1)?;
        visited.insert(node.id());
        match node.kind() {
            ProvenanceKindRef::Source(source) => {
                sources.try_reserve(1)?;
                sources.insert(source.index());
                // Source indices are local to this borrowed Scene; no persistent key is minted.
                assert_eq!(source.generation(), scene.generation());
                let _ = source.resource().identity();
            }
            ProvenanceKindRef::Operation(_) => {
                let parents = node.parents();
                pending.try_reserve(parents.len())?;
                pending.extend(parents);
            }
        }
    }
    metrics.traced_provenance_nodes = visited.len();
    metrics.traced_source_objects = sources.len();
    Ok(())
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
