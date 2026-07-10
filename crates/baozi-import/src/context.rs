use crate::format::FormatInfo;
use crate::outcome::{ImportFailure, ImportResult};
use crate::registry::DetectionEvidence;
use baozi_core::{
    BaoziError, Diagnostic, MaterialProperty, MetadataMap, MetadataValue, Result, Scene,
    TextureSource, VertexAttributeSemantic, validate_scene,
};
use baozi_io::{AssetIo, AssetPath, ResourceLimits};
use std::io::Read;

#[derive(Debug, Clone, Default)]
pub struct ReadHint {
    pub source: Option<AssetPath>,
    pub extension: Option<String>,
}

impl ReadHint {
    pub fn from_source(source: AssetPath) -> Self {
        let extension = source.extension().map(str::to_owned);
        Self {
            source: Some(source),
            extension,
        }
    }

    pub fn display_hint(&self) -> String {
        self.extension
            .as_deref()
            .or_else(|| self.source.as_ref().map(AssetPath::as_str))
            .filter(|hint| !hint.is_empty())
            .unwrap_or("unknown")
            .to_owned()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalReferencePolicy {
    Deny,
    WithinScope,
    AllowListedRoots,
    CustomResolver,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IoOptions {
    pub external_references: ExternalReferencePolicy,
}

impl IoOptions {
    pub const fn memory() -> Self {
        Self {
            external_references: ExternalReferencePolicy::Deny,
        }
    }

    pub const fn filesystem() -> Self {
        Self {
            external_references: ExternalReferencePolicy::WithinScope,
        }
    }
}

impl Default for IoOptions {
    fn default() -> Self {
        Self::memory()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectionOptions {
    pub max_probe_bytes: u64,
}

impl Default for DetectionOptions {
    fn default() -> Self {
        Self {
            max_probe_bytes: 4096,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiagnosticOptions {
    pub strict: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ImportOptions {
    pub limits: ResourceLimits,
    pub io: IoOptions,
    pub detection: DetectionOptions,
    pub diagnostics: DiagnosticOptions,
}

impl ImportOptions {
    pub fn memory() -> Self {
        Self::default()
    }

    pub fn filesystem() -> Self {
        Self {
            io: IoOptions::filesystem(),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportStage {
    ValidatedImported,
    PostProcessed,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportStats {
    primary_asset_bytes: u64,
    sidecar_asset_bytes: u64,
    data_uri_bytes: u64,
    total_asset_bytes: u64,
    opened_assets: usize,
    graph_nodes: usize,
    graph_edges: usize,
    graph_work: usize,
    vertex_attribute_streams: usize,
    vertex_attribute_cells: usize,
    topology_cells: usize,
    canonical_string_bytes: usize,
    generated_meshes: usize,
    generated_vertices: usize,
    generated_faces: usize,
    generated_materials: usize,
    generated_textures: usize,
    diagnostics_emitted: usize,
    diagnostics_dropped: usize,
}

impl ImportStats {
    pub const fn primary_asset_bytes(&self) -> u64 {
        self.primary_asset_bytes
    }

    pub const fn sidecar_asset_bytes(&self) -> u64 {
        self.sidecar_asset_bytes
    }

    pub const fn data_uri_bytes(&self) -> u64 {
        self.data_uri_bytes
    }

    pub const fn total_asset_bytes(&self) -> u64 {
        self.total_asset_bytes
    }

    pub const fn opened_assets(&self) -> usize {
        self.opened_assets
    }

    pub const fn graph_nodes(&self) -> usize {
        self.graph_nodes
    }

    pub const fn graph_edges(&self) -> usize {
        self.graph_edges
    }

    pub const fn graph_work(&self) -> usize {
        self.graph_work
    }

    pub const fn vertex_attribute_streams(&self) -> usize {
        self.vertex_attribute_streams
    }

    pub const fn vertex_attribute_cells(&self) -> usize {
        self.vertex_attribute_cells
    }

    pub const fn topology_cells(&self) -> usize {
        self.topology_cells
    }

    pub const fn canonical_string_bytes(&self) -> usize {
        self.canonical_string_bytes
    }

    pub const fn generated_meshes(&self) -> usize {
        self.generated_meshes
    }

    pub const fn generated_vertices(&self) -> usize {
        self.generated_vertices
    }

    pub const fn generated_faces(&self) -> usize {
        self.generated_faces
    }

    pub const fn generated_materials(&self) -> usize {
        self.generated_materials
    }

    pub const fn generated_textures(&self) -> usize {
        self.generated_textures
    }

    pub const fn diagnostics_emitted(&self) -> usize {
        self.diagnostics_emitted
    }

    pub const fn diagnostics_dropped(&self) -> usize {
        self.diagnostics_dropped
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SceneCounts {
    nodes: usize,
    graph_edges: usize,
    graph_work: usize,
    vertex_attribute_streams: usize,
    vertex_attribute_cells: usize,
    topology_cells: usize,
    canonical_string_bytes: usize,
    meshes: usize,
    vertices: usize,
    faces: usize,
    materials: usize,
    textures: usize,
}

impl SceneCounts {
    fn from_scene(scene: &Scene, limits: &ResourceLimits) -> Result<Self> {
        let nodes = scene.nodes.len();
        if nodes > limits.max_graph_nodes {
            return Err(BaoziError::LimitExceeded {
                limit: "max_graph_nodes",
            });
        }
        let meshes = scene.meshes.len();
        if meshes > limits.max_meshes {
            return Err(BaoziError::LimitExceeded {
                limit: "max_meshes",
            });
        }

        let mut graph_edges = 0usize;
        for node in &scene.nodes {
            graph_edges =
                graph_edges
                    .checked_add(node.children.len())
                    .ok_or(BaoziError::LimitExceeded {
                        limit: "max_graph_edges",
                    })?;
            if graph_edges > limits.max_graph_edges {
                return Err(BaoziError::LimitExceeded {
                    limit: "max_graph_edges",
                });
            }
            let graph_work = nodes
                .checked_add(graph_edges)
                .ok_or(BaoziError::LimitExceeded {
                    limit: "max_graph_work",
                })?;
            if graph_work > limits.max_graph_work {
                return Err(BaoziError::LimitExceeded {
                    limit: "max_graph_work",
                });
            }
        }
        let graph_work = nodes
            .checked_add(graph_edges)
            .ok_or(BaoziError::LimitExceeded {
                limit: "max_graph_work",
            })?;

        let mut vertices = 0usize;
        let mut faces = 0usize;
        let mut vertex_attribute_streams = 0usize;
        let mut vertex_attribute_cells = 0usize;
        let mut topology_cells = 0usize;
        for mesh in &scene.meshes {
            vertices =
                vertices
                    .checked_add(mesh.positions.len())
                    .ok_or(BaoziError::LimitExceeded {
                        limit: "max_vertices",
                    })?;
            if vertices > limits.max_vertices {
                return Err(BaoziError::LimitExceeded {
                    limit: "max_vertices",
                });
            }

            faces = faces
                .checked_add(mesh_face_count(mesh))
                .ok_or(BaoziError::LimitExceeded { limit: "max_faces" })?;
            if faces > limits.max_faces {
                return Err(BaoziError::LimitExceeded { limit: "max_faces" });
            }

            let mesh_topology_cells = mesh
                .indices
                .len()
                .checked_add(mesh.face_vertex_counts.len())
                .ok_or(BaoziError::LimitExceeded {
                    limit: "max_total_topology_cells",
                })?;
            topology_cells = topology_cells.checked_add(mesh_topology_cells).ok_or(
                BaoziError::LimitExceeded {
                    limit: "max_total_topology_cells",
                },
            )?;
            if topology_cells > limits.max_total_topology_cells {
                return Err(BaoziError::LimitExceeded {
                    limit: "max_total_topology_cells",
                });
            }

            count_mesh_vertex_attributes(
                mesh,
                limits,
                &mut vertex_attribute_streams,
                &mut vertex_attribute_cells,
            )?;
        }
        let canonical_string_bytes = count_canonical_string_bytes(scene, limits)?;

        Ok(Self {
            nodes,
            graph_edges,
            graph_work,
            vertex_attribute_streams,
            vertex_attribute_cells,
            topology_cells,
            canonical_string_bytes,
            meshes,
            vertices,
            faces,
            materials: scene.materials.len(),
            textures: scene.textures.len(),
        })
    }

    fn apply_to(self, stats: &mut ImportStats) {
        stats.graph_nodes = self.nodes;
        stats.graph_edges = self.graph_edges;
        stats.graph_work = self.graph_work;
        stats.vertex_attribute_streams = self.vertex_attribute_streams;
        stats.vertex_attribute_cells = self.vertex_attribute_cells;
        stats.topology_cells = self.topology_cells;
        stats.canonical_string_bytes = self.canonical_string_bytes;
        stats.generated_meshes = self.meshes;
        stats.generated_vertices = self.vertices;
        stats.generated_faces = self.faces;
        stats.generated_materials = self.materials;
        stats.generated_textures = self.textures;
    }
}

fn count_mesh_vertex_attributes(
    mesh: &baozi_core::Mesh,
    limits: &ResourceLimits,
    total_streams: &mut usize,
    total_cells: &mut usize,
) -> Result<()> {
    let mut mesh_streams = 0usize;
    for len in [
        mesh.positions.len(),
        mesh.normals.len(),
        mesh.tangents.len(),
        mesh.joint_indices.len(),
        mesh.joint_weights.len(),
    ] {
        if len != 0 {
            count_vertex_attribute_stream(
                len,
                limits,
                &mut mesh_streams,
                total_streams,
                total_cells,
            )?;
        }
    }
    for channel in &mesh.texcoords {
        count_vertex_attribute_stream(
            channel.len(),
            limits,
            &mut mesh_streams,
            total_streams,
            total_cells,
        )?;
    }
    for channel in &mesh.colors {
        count_vertex_attribute_stream(
            channel.len(),
            limits,
            &mut mesh_streams,
            total_streams,
            total_cells,
        )?;
    }
    for target in &mesh.morph_targets {
        for len in [
            target.positions.len(),
            target.normals.len(),
            target.tangents.len(),
        ] {
            if len != 0 {
                count_vertex_attribute_stream(
                    len,
                    limits,
                    &mut mesh_streams,
                    total_streams,
                    total_cells,
                )?;
            }
        }
    }
    for attribute in &mesh.custom_attributes {
        count_vertex_attribute_stream(
            attribute.data.len(),
            limits,
            &mut mesh_streams,
            total_streams,
            total_cells,
        )?;
    }
    Ok(())
}

fn count_vertex_attribute_stream(
    len: usize,
    limits: &ResourceLimits,
    mesh_streams: &mut usize,
    total_streams: &mut usize,
    total_cells: &mut usize,
) -> Result<()> {
    *mesh_streams = mesh_streams
        .checked_add(1)
        .ok_or(BaoziError::LimitExceeded {
            limit: "max_vertex_attribute_streams_per_mesh",
        })?;
    if *mesh_streams > limits.max_vertex_attribute_streams_per_mesh {
        return Err(BaoziError::LimitExceeded {
            limit: "max_vertex_attribute_streams_per_mesh",
        });
    }
    *total_streams = total_streams
        .checked_add(1)
        .ok_or(BaoziError::LimitExceeded {
            limit: "max_total_vertex_attribute_streams",
        })?;
    if *total_streams > limits.max_total_vertex_attribute_streams {
        return Err(BaoziError::LimitExceeded {
            limit: "max_total_vertex_attribute_streams",
        });
    }
    *total_cells = total_cells
        .checked_add(len)
        .ok_or(BaoziError::LimitExceeded {
            limit: "max_total_vertex_attribute_cells",
        })?;
    if *total_cells > limits.max_total_vertex_attribute_cells {
        return Err(BaoziError::LimitExceeded {
            limit: "max_total_vertex_attribute_cells",
        });
    }
    Ok(())
}

struct CanonicalStringCounter {
    bytes: usize,
    limit: usize,
}

impl CanonicalStringCounter {
    const LIMIT_NAME: &'static str = "max_canonical_string_bytes";

    const fn new(limit: usize) -> Self {
        Self { bytes: 0, limit }
    }

    fn add(&mut self, value: &str) -> Result<()> {
        let next = self
            .bytes
            .checked_add(value.len())
            .ok_or(BaoziError::LimitExceeded {
                limit: Self::LIMIT_NAME,
            })?;
        if next > self.limit {
            return Err(BaoziError::LimitExceeded {
                limit: Self::LIMIT_NAME,
            });
        }
        self.bytes = next;
        Ok(())
    }

    fn add_optional(&mut self, value: Option<&str>) -> Result<()> {
        if let Some(value) = value {
            self.add(value)?;
        }
        Ok(())
    }

    fn add_metadata(&mut self, metadata: &MetadataMap) -> Result<()> {
        for (key, value) in metadata {
            self.add(key)?;
            if let MetadataValue::String(value) = value {
                self.add(value)?;
            }
        }
        Ok(())
    }
}

fn count_canonical_string_bytes(scene: &Scene, limits: &ResourceLimits) -> Result<usize> {
    let mut counter = CanonicalStringCounter::new(limits.max_canonical_string_bytes);
    counter.add_metadata(&scene.metadata)?;

    for node in &scene.nodes {
        counter.add_optional(node.name.as_deref())?;
        counter.add_metadata(&node.metadata)?;
    }

    for mesh in &scene.meshes {
        counter.add_optional(mesh.name.as_deref())?;
        counter.add_metadata(&mesh.metadata)?;
        for target in &mesh.morph_targets {
            counter.add_optional(target.name.as_deref())?;
            counter.add_metadata(&target.metadata)?;
        }
        for attribute in &mesh.custom_attributes {
            counter.add(&attribute.name)?;
            if let VertexAttributeSemantic::Custom(semantic) = &attribute.semantic {
                counter.add(semantic)?;
            }
            counter.add_metadata(&attribute.metadata)?;
        }
    }

    for animation in &scene.animations {
        counter.add_optional(animation.name.as_deref())?;
        counter.add_metadata(&animation.metadata)?;
        for channel in &animation.channels {
            counter.add_metadata(&channel.metadata)?;
        }
    }

    for camera in &scene.cameras {
        counter.add_optional(camera.name.as_deref())?;
        counter.add_metadata(&camera.metadata)?;
    }
    for light in &scene.lights {
        counter.add_optional(light.name.as_deref())?;
        counter.add_metadata(&light.metadata)?;
    }
    for skin in &scene.skins {
        counter.add_optional(skin.name.as_deref())?;
        counter.add_metadata(&skin.metadata)?;
    }

    for material in &scene.materials {
        counter.add_optional(material.name.as_deref())?;
        for (key, value) in &material.properties {
            counter.add(key)?;
            if let MaterialProperty::String(value) = value {
                counter.add(value)?;
            }
        }
        for slot in &material.texture_slots {
            counter.add_optional(slot.source_key.as_deref())?;
        }
        counter.add_metadata(&material.metadata)?;
    }

    for texture in &scene.textures {
        counter.add_optional(texture.name.as_deref())?;
        match &texture.source {
            TextureSource::External { uri } => counter.add(uri)?,
            TextureSource::Embedded { mime_type, .. } => {
                counter.add_optional(mime_type.as_deref())?;
            }
        }
        counter.add_metadata(&texture.metadata)?;
    }

    Ok(counter.bytes)
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResourceLedger {
    stats: ImportStats,
}

impl ResourceLedger {
    pub fn stats(&self) -> &ImportStats {
        &self.stats
    }

    fn debit_open_asset(&mut self, limits: &ResourceLimits) -> Result<()> {
        let next = self
            .stats
            .opened_assets
            .checked_add(1)
            .ok_or(BaoziError::LimitExceeded {
                limit: "max_open_assets",
            })?;
        if next > limits.max_open_assets {
            return Err(BaoziError::LimitExceeded {
                limit: "max_open_assets",
            });
        }
        self.stats.opened_assets = next;
        Ok(())
    }

    fn debit_primary_bytes(&mut self, bytes: u64, limits: &ResourceLimits) -> Result<()> {
        let primary =
            self.stats
                .primary_asset_bytes
                .checked_add(bytes)
                .ok_or(BaoziError::LimitExceeded {
                    limit: "max_primary_asset_bytes",
                })?;
        let total = self.checked_total_bytes(bytes)?;
        self.stats.primary_asset_bytes = primary;
        self.stats.total_asset_bytes = total;
        if primary > limits.max_primary_asset_bytes {
            return Err(BaoziError::LimitExceeded {
                limit: "max_primary_asset_bytes",
            });
        }
        if total > limits.max_total_asset_bytes {
            return Err(BaoziError::LimitExceeded {
                limit: "max_total_asset_bytes",
            });
        }
        Ok(())
    }

    fn debit_sidecar_bytes(&mut self, bytes: u64, limits: &ResourceLimits) -> Result<()> {
        let sidecar =
            self.stats
                .sidecar_asset_bytes
                .checked_add(bytes)
                .ok_or(BaoziError::LimitExceeded {
                    limit: "max_sidecar_asset_bytes",
                })?;
        let total = self.checked_total_bytes(bytes)?;
        self.stats.sidecar_asset_bytes = sidecar;
        self.stats.total_asset_bytes = total;
        if bytes > limits.max_sidecar_asset_bytes {
            return Err(BaoziError::LimitExceeded {
                limit: "max_sidecar_asset_bytes",
            });
        }
        if total > limits.max_total_asset_bytes {
            return Err(BaoziError::LimitExceeded {
                limit: "max_total_asset_bytes",
            });
        }
        Ok(())
    }

    fn check_data_uri_bytes(&self, bytes: u64, limits: &ResourceLimits) -> Result<()> {
        let data_uri =
            self.stats
                .data_uri_bytes
                .checked_add(bytes)
                .ok_or(BaoziError::LimitExceeded {
                    limit: "max_data_uri_bytes",
                })?;
        if data_uri > limits.max_data_uri_bytes {
            return Err(BaoziError::LimitExceeded {
                limit: "max_data_uri_bytes",
            });
        }
        let total = self.checked_total_bytes(bytes)?;
        if total > limits.max_total_asset_bytes {
            return Err(BaoziError::LimitExceeded {
                limit: "max_total_asset_bytes",
            });
        }
        Ok(())
    }

    fn debit_data_uri_bytes(&mut self, bytes: u64, limits: &ResourceLimits) -> Result<()> {
        self.check_data_uri_bytes(bytes, limits)?;
        let data_uri =
            self.stats
                .data_uri_bytes
                .checked_add(bytes)
                .ok_or(BaoziError::LimitExceeded {
                    limit: "max_data_uri_bytes",
                })?;
        let total = self.checked_total_bytes(bytes)?;
        self.stats.data_uri_bytes = data_uri;
        self.stats.total_asset_bytes = total;
        Ok(())
    }

    fn checked_total_bytes(&self, bytes: u64) -> Result<u64> {
        self.stats
            .total_asset_bytes
            .checked_add(bytes)
            .ok_or(BaoziError::LimitExceeded {
                limit: "max_total_asset_bytes",
            })
    }

    fn remaining_total_bytes(&self, limits: &ResourceLimits) -> Result<u64> {
        limits
            .max_total_asset_bytes
            .checked_sub(self.stats.total_asset_bytes)
            .ok_or(BaoziError::LimitExceeded {
                limit: "max_total_asset_bytes",
            })
    }

    fn record_scene_counts(&mut self, counts: SceneCounts) {
        counts.apply_to(&mut self.stats);
    }

    fn record_diagnostic(&mut self, stored: bool) {
        if stored {
            self.stats.diagnostics_emitted = self.stats.diagnostics_emitted.saturating_add(1);
        } else {
            self.stats.diagnostics_dropped = self.stats.diagnostics_dropped.saturating_add(1);
        }
    }

    fn into_stats(self) -> ImportStats {
        self.stats
    }
}

fn mesh_face_count(mesh: &baozi_core::Mesh) -> usize {
    match mesh.topology {
        baozi_core::PrimitiveTopology::Triangles => mesh.element_count() / 3,
        baozi_core::PrimitiveTopology::Polygons => mesh.face_vertex_counts.len(),
        baozi_core::PrimitiveTopology::Points | baozi_core::PrimitiveTopology::Lines => 0,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportReport {
    scene: Scene,
    diagnostics: Vec<Diagnostic>,
    format: FormatInfo,
    detection_evidence: Vec<DetectionEvidence>,
    limits: ResourceLimits,
    stage: ImportStage,
    stats: ImportStats,
}

impl ImportReport {
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn format(&self) -> &FormatInfo {
        &self.format
    }

    pub fn detection_evidence(&self) -> &[DetectionEvidence] {
        &self.detection_evidence
    }

    pub fn limits(&self) -> &ResourceLimits {
        &self.limits
    }

    pub fn stage(&self) -> ImportStage {
        self.stage
    }

    pub fn stats(&self) -> &ImportStats {
        &self.stats
    }

    pub fn apply_postprocess(
        mut self,
        process: impl FnOnce(Scene) -> Result<Scene>,
    ) -> ImportResult<Self> {
        let detection_evidence = self.detection_evidence.clone();
        if self.stage != ImportStage::ValidatedImported {
            return Err(ImportFailure::operation(
                BaoziError::PostProcess {
                    step: "report transition",
                    message: "postprocess can only run on a validated imported scene".to_owned(),
                },
                detection_evidence,
            ));
        }
        self.scene = process(self.scene)
            .map_err(|error| ImportFailure::operation(error, detection_evidence.clone()))?;
        let counts = validate_scene_for_report(&self.scene, &self.limits)
            .map_err(|error| ImportFailure::operation(error, detection_evidence))?;
        counts.apply_to(&mut self.stats);
        self.stage = ImportStage::PostProcessed;
        Ok(self)
    }
}

pub struct ImportContext<'a> {
    io: &'a dyn AssetIo,
    source: AssetPath,
    options: ImportOptions,
    diagnostics: Vec<Diagnostic>,
    strict_violation: Option<Diagnostic>,
    ledger: ResourceLedger,
}

struct AssetReadFailure {
    error: BaoziError,
    bytes_read: u64,
}

impl<'a> ImportContext<'a> {
    pub fn new(io: &'a dyn AssetIo, source: AssetPath) -> Self {
        Self::with_options(io, source, ImportOptions::default())
    }

    pub fn with_options(io: &'a dyn AssetIo, source: AssetPath, options: ImportOptions) -> Self {
        Self {
            io,
            source,
            options,
            diagnostics: Vec::new(),
            strict_violation: None,
            ledger: ResourceLedger::default(),
        }
    }

    pub fn source(&self) -> &AssetPath {
        &self.source
    }

    pub fn limits(&self) -> &ResourceLimits {
        &self.options.limits
    }

    pub fn io_options(&self) -> &IoOptions {
        &self.options.io
    }

    pub fn diagnostic_options(&self) -> &DiagnosticOptions {
        &self.options.diagnostics
    }

    pub fn ledger(&self) -> &ResourceLedger {
        &self.ledger
    }

    pub fn push_diagnostic(&mut self, diagnostic: Diagnostic) {
        if diagnostic.severity != baozi_core::DiagnosticSeverity::Info
            && self.strict_violation.is_none()
        {
            self.strict_violation = Some(diagnostic.clone());
        }
        if self.diagnostics.len() < self.options.limits.max_diagnostics {
            self.diagnostics.push(diagnostic);
            self.ledger.record_diagnostic(true);
        } else {
            self.ledger.record_diagnostic(false);
        }
    }

    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    pub fn finish_import(
        mut self,
        scene: Scene,
        format: FormatInfo,
        detection_evidence: Vec<DetectionEvidence>,
    ) -> Result<ImportReport> {
        let counts = validate_scene_for_report(&scene, &self.options.limits)?;
        self.ledger.record_scene_counts(counts);
        if self.options.diagnostics.strict
            && let Some(diagnostic) = self.strict_violation.as_ref()
        {
            return Err(BaoziError::parse(
                diagnostic
                    .source
                    .clone()
                    .unwrap_or_else(|| self.source.to_string()),
                diagnostic.location,
                format!("{}: {}", diagnostic.code.0, diagnostic.message),
            ));
        }
        Ok(ImportReport {
            scene,
            diagnostics: self.diagnostics,
            format,
            detection_evidence,
            limits: self.options.limits,
            stage: ImportStage::ValidatedImported,
            stats: self.ledger.into_stats(),
        })
    }

    pub fn read_primary_to_end(&mut self) -> Result<Vec<u8>> {
        let source = self.source.clone();
        let (limit, limit_name) = self.effective_asset_read_limit(
            self.options.limits.max_primary_asset_bytes,
            "max_primary_asset_bytes",
        )?;
        match self.read_asset_to_end(&source, limit, limit_name) {
            Ok(bytes) => {
                self.ledger
                    .debit_primary_bytes(bytes.len() as u64, &self.options.limits)?;
                Ok(bytes)
            }
            Err(failure) => {
                self.ledger
                    .debit_primary_bytes(failure.bytes_read, &self.options.limits)?;
                Err(failure.error)
            }
        }
    }

    pub fn read_sidecar_to_end(&mut self, path: &AssetPath) -> Result<Vec<u8>> {
        self.ensure_external_references_allowed(path.as_str())?;
        let (limit, limit_name) = self.effective_asset_read_limit(
            self.options.limits.max_sidecar_asset_bytes,
            "max_sidecar_asset_bytes",
        )?;
        match self.read_asset_to_end(path, limit, limit_name) {
            Ok(bytes) => {
                self.ledger
                    .debit_sidecar_bytes(bytes.len() as u64, &self.options.limits)?;
                Ok(bytes)
            }
            Err(failure) => {
                self.ledger
                    .debit_sidecar_bytes(failure.bytes_read, &self.options.limits)?;
                Err(failure.error)
            }
        }
    }

    pub fn check_data_uri_bytes(&self, bytes: u64) -> Result<()> {
        self.ledger
            .check_data_uri_bytes(bytes, &self.options.limits)
    }

    pub fn record_data_uri_bytes(&mut self, bytes: u64) -> Result<()> {
        self.ledger
            .debit_data_uri_bytes(bytes, &self.options.limits)
    }

    pub fn resolve_source_relative(&self, relative: &str) -> Result<AssetPath> {
        self.ensure_external_references_allowed(relative)?;
        self.io.resolve(&self.source, relative)
    }

    pub fn resolve_relative(&self, base: &AssetPath, relative: &str) -> Result<AssetPath> {
        self.ensure_external_references_allowed(relative)?;
        self.io.resolve(base, relative)
    }

    fn ensure_external_references_allowed(&self, asset: &str) -> Result<()> {
        if matches!(
            self.options.io.external_references,
            ExternalReferencePolicy::Deny
        ) {
            return Err(BaoziError::io(
                asset,
                "external references are denied by import policy",
            ));
        }
        Ok(())
    }

    fn effective_asset_read_limit(
        &self,
        per_asset_limit: u64,
        per_asset_limit_name: &'static str,
    ) -> Result<(u64, &'static str)> {
        let remaining_total = self.ledger.remaining_total_bytes(&self.options.limits)?;
        if remaining_total == 0 {
            return Err(BaoziError::LimitExceeded {
                limit: "max_total_asset_bytes",
            });
        }
        if remaining_total < per_asset_limit {
            Ok((remaining_total, "max_total_asset_bytes"))
        } else {
            Ok((per_asset_limit, per_asset_limit_name))
        }
    }

    fn read_asset_to_end(
        &mut self,
        path: &AssetPath,
        limit: u64,
        limit_name: &'static str,
    ) -> std::result::Result<Vec<u8>, AssetReadFailure> {
        self.ledger
            .debit_open_asset(&self.options.limits)
            .map_err(|error| AssetReadFailure {
                error,
                bytes_read: 0,
            })?;
        let mut reader = self.io.open(path).map_err(|error| AssetReadFailure {
            error,
            bytes_read: 0,
        })?;
        let mut bytes = Vec::new();
        let mut limited = reader.by_ref().take(limit.saturating_add(1));
        if let Err(error) = limited.read_to_end(&mut bytes) {
            return Err(AssetReadFailure {
                error: BaoziError::io(path.to_string(), error.to_string()),
                bytes_read: bytes.len() as u64,
            });
        }
        if bytes.len() as u64 > limit {
            return Err(AssetReadFailure {
                error: BaoziError::LimitExceeded { limit: limit_name },
                bytes_read: bytes.len() as u64,
            });
        }
        Ok(bytes)
    }
}

fn validate_scene_for_report(scene: &Scene, limits: &ResourceLimits) -> Result<SceneCounts> {
    let counts = SceneCounts::from_scene(scene, limits)?;
    validate_scene(scene)?;
    validate_graph_depth(scene, limits)?;
    Ok(counts)
}

fn validate_graph_depth(scene: &Scene, limits: &ResourceLimits) -> Result<()> {
    let mut pending = Vec::new();
    pending
        .try_reserve_exact(scene.nodes.len())
        .map_err(|_| BaoziError::LimitExceeded {
            limit: "max_graph_nodes",
        })?;
    pending.push((scene.root.index(), 1usize));

    while let Some((node_index, depth)) = pending.pop() {
        if depth > limits.max_graph_depth {
            return Err(BaoziError::LimitExceeded {
                limit: "max_graph_depth",
            });
        }
        let child_depth = depth.checked_add(1).ok_or(BaoziError::LimitExceeded {
            limit: "max_graph_depth",
        })?;
        pending.extend(
            scene.nodes[node_index]
                .children
                .iter()
                .rev()
                .map(|child| (child.index(), child_depth)),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use baozi_io::{MemoryAssetIo, ReadSeek};
    use std::io::{self, Cursor, Seek, SeekFrom};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug, Default)]
    struct RecordingAssetIo {
        open_calls: AtomicUsize,
        resolve_calls: AtomicUsize,
    }

    #[derive(Debug, Default)]
    struct PartialReadAssetIo {
        open_calls: AtomicUsize,
    }

    struct PartialErrorReader {
        emitted: bool,
    }

    impl Read for PartialErrorReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.emitted {
                return Err(io::Error::other("sidecar read failed"));
            }
            let bytes = [1_u8, 2, 3];
            let count = buffer.len().min(bytes.len());
            buffer[..count].copy_from_slice(&bytes[..count]);
            self.emitted = true;
            Ok(count)
        }
    }

    impl Seek for PartialErrorReader {
        fn seek(&mut self, _position: SeekFrom) -> io::Result<u64> {
            Ok(0)
        }
    }

    impl AssetIo for RecordingAssetIo {
        fn open(&self, _path: &AssetPath) -> Result<Box<dyn ReadSeek + Send>> {
            self.open_calls.fetch_add(1, Ordering::Relaxed);
            Ok(Box::new(Cursor::new(Vec::<u8>::new())))
        }

        fn exists(&self, _path: &AssetPath) -> bool {
            false
        }

        fn resolve(&self, base: &AssetPath, relative: &str) -> Result<AssetPath> {
            self.resolve_calls.fetch_add(1, Ordering::Relaxed);
            base.join(relative)
        }
    }

    impl AssetIo for PartialReadAssetIo {
        fn open(&self, _path: &AssetPath) -> Result<Box<dyn ReadSeek + Send>> {
            self.open_calls.fetch_add(1, Ordering::Relaxed);
            Ok(Box::new(PartialErrorReader { emitted: false }))
        }

        fn exists(&self, _path: &AssetPath) -> bool {
            true
        }

        fn resolve(&self, base: &AssetPath, relative: &str) -> Result<AssetPath> {
            base.join(relative)
        }
    }

    #[test]
    fn denied_sidecars_are_rejected_before_backend_io() {
        let io = RecordingAssetIo::default();
        let source = AssetPath::new("model.obj").expect("valid source path");
        let sidecar = AssetPath::new("material.mtl").expect("valid sidecar path");
        let mut ctx = ImportContext::new(&io, source);

        let resolve_error = ctx
            .resolve_source_relative("material.mtl")
            .expect_err("denied sidecar resolution must fail");
        assert!(matches!(resolve_error, BaoziError::Io { .. }));

        let nested_resolve_error = ctx
            .resolve_relative(&sidecar, "texture.png")
            .expect_err("nested sidecar resolution must fail");
        assert!(matches!(nested_resolve_error, BaoziError::Io { .. }));

        let read_error = ctx
            .read_sidecar_to_end(&sidecar)
            .expect_err("denied sidecar reads must fail");
        assert!(matches!(read_error, BaoziError::Io { .. }));

        assert_eq!(io.resolve_calls.load(Ordering::Relaxed), 0);
        assert_eq!(io.open_calls.load(Ordering::Relaxed), 0);
        assert_eq!(ctx.ledger().stats().opened_assets(), 0);
    }

    #[test]
    fn oversized_sidecar_attempt_is_not_refunded() {
        let source = AssetPath::new("model.obj").expect("valid source path");
        let sidecar = AssetPath::new("material.mtl").expect("valid sidecar path");
        let mut io = MemoryAssetIo::new();
        io.insert(sidecar.clone(), vec![0_u8; 10]);
        let mut options = ImportOptions::filesystem();
        options.limits.max_sidecar_asset_bytes = 4;
        options.limits.max_total_asset_bytes = 4;
        let mut ctx = ImportContext::with_options(&io, source, options);

        let error = ctx
            .read_sidecar_to_end(&sidecar)
            .expect_err("oversized sidecar must fail");
        assert_eq!(
            error,
            BaoziError::LimitExceeded {
                limit: "max_sidecar_asset_bytes"
            }
        );
        assert_eq!(ctx.ledger().stats().sidecar_asset_bytes(), 5);
        assert_eq!(ctx.ledger().stats().total_asset_bytes(), 5);
        assert_eq!(ctx.ledger().stats().opened_assets(), 1);

        let error = ctx
            .read_sidecar_to_end(&sidecar)
            .expect_err("spent total budget must fail before another open");
        assert_eq!(
            error,
            BaoziError::LimitExceeded {
                limit: "max_total_asset_bytes"
            }
        );
        assert_eq!(ctx.ledger().stats().opened_assets(), 1);
    }

    #[test]
    fn partial_io_failure_consumes_the_bytes_already_read() {
        let io = PartialReadAssetIo::default();
        let source = AssetPath::new("model.obj").expect("valid source path");
        let sidecar = AssetPath::new("material.mtl").expect("valid sidecar path");
        let mut options = ImportOptions::filesystem();
        options.limits.max_sidecar_asset_bytes = 10;
        options.limits.max_total_asset_bytes = 3;
        let mut ctx = ImportContext::with_options(&io, source, options);

        let error = ctx
            .read_sidecar_to_end(&sidecar)
            .expect_err("partial sidecar read must preserve its IO error");
        assert!(matches!(error, BaoziError::Io { .. }));
        assert_eq!(ctx.ledger().stats().sidecar_asset_bytes(), 3);
        assert_eq!(ctx.ledger().stats().total_asset_bytes(), 3);
        assert_eq!(io.open_calls.load(Ordering::Relaxed), 1);

        let error = ctx
            .read_sidecar_to_end(&sidecar)
            .expect_err("spent total budget must stop repeated partial reads");
        assert_eq!(
            error,
            BaoziError::LimitExceeded {
                limit: "max_total_asset_bytes"
            }
        );
        assert_eq!(io.open_calls.load(Ordering::Relaxed), 1);
        assert_eq!(ctx.ledger().stats().opened_assets(), 1);
    }
}
