use crate::format::FormatInfo;
use baozi_core::{BaoziError, Diagnostic, Result, Scene};
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
    total_asset_bytes: u64,
    opened_assets: usize,
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

    pub const fn total_asset_bytes(&self) -> u64 {
        self.total_asset_bytes
    }

    pub const fn opened_assets(&self) -> usize {
        self.opened_assets
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

    pub fn record_scene_counts(&mut self, scene: &Scene) -> Result<()> {
        self.generated_meshes = scene.meshes.len();
        self.generated_vertices = scene
            .meshes
            .iter()
            .try_fold(0usize, |total, mesh| {
                total.checked_add(mesh.positions.len())
            })
            .ok_or(BaoziError::LimitExceeded {
                limit: "max_vertices",
            })?;
        self.generated_faces = scene
            .meshes
            .iter()
            .try_fold(0usize, |total, mesh| {
                total.checked_add(mesh_face_count(mesh))
            })
            .ok_or(BaoziError::LimitExceeded { limit: "max_faces" })?;
        self.generated_materials = scene.materials.len();
        self.generated_textures = scene.textures.len();
        Ok(())
    }
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
        if bytes > limits.max_primary_asset_bytes {
            return Err(BaoziError::LimitExceeded {
                limit: "max_primary_asset_bytes",
            });
        }
        self.stats.primary_asset_bytes =
            self.stats
                .primary_asset_bytes
                .checked_add(bytes)
                .ok_or(BaoziError::LimitExceeded {
                    limit: "max_primary_asset_bytes",
                })?;
        self.debit_total_bytes(bytes, limits)
    }

    fn debit_sidecar_bytes(&mut self, bytes: u64, limits: &ResourceLimits) -> Result<()> {
        if bytes > limits.max_sidecar_asset_bytes {
            return Err(BaoziError::LimitExceeded {
                limit: "max_sidecar_asset_bytes",
            });
        }
        self.stats.sidecar_asset_bytes =
            self.stats
                .sidecar_asset_bytes
                .checked_add(bytes)
                .ok_or(BaoziError::LimitExceeded {
                    limit: "max_sidecar_asset_bytes",
                })?;
        self.debit_total_bytes(bytes, limits)
    }

    fn debit_total_bytes(&mut self, bytes: u64, limits: &ResourceLimits) -> Result<()> {
        let next =
            self.stats
                .total_asset_bytes
                .checked_add(bytes)
                .ok_or(BaoziError::LimitExceeded {
                    limit: "max_total_asset_bytes",
                })?;
        if next > limits.max_total_asset_bytes {
            return Err(BaoziError::LimitExceeded {
                limit: "max_total_asset_bytes",
            });
        }
        self.stats.total_asset_bytes = next;
        Ok(())
    }

    fn record_scene(&mut self, scene: &Scene, limits: &ResourceLimits) -> Result<()> {
        let vertices = scene
            .meshes
            .iter()
            .try_fold(0usize, |total, mesh| {
                total.checked_add(mesh.positions.len())
            })
            .ok_or(BaoziError::LimitExceeded {
                limit: "max_vertices",
            })?;
        let faces = scene
            .meshes
            .iter()
            .try_fold(0usize, |total, mesh| {
                total.checked_add(mesh_face_count(mesh))
            })
            .ok_or(BaoziError::LimitExceeded { limit: "max_faces" })?;

        if scene.meshes.len() > limits.max_meshes {
            return Err(BaoziError::LimitExceeded {
                limit: "max_meshes",
            });
        }
        if vertices > limits.max_vertices {
            return Err(BaoziError::LimitExceeded {
                limit: "max_vertices",
            });
        }
        if faces > limits.max_faces {
            return Err(BaoziError::LimitExceeded { limit: "max_faces" });
        }

        self.stats.generated_meshes = scene.meshes.len();
        self.stats.generated_vertices = vertices;
        self.stats.generated_faces = faces;
        self.stats.generated_materials = scene.materials.len();
        self.stats.generated_textures = scene.textures.len();
        Ok(())
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

    pub fn stage(&self) -> ImportStage {
        self.stage
    }

    pub fn stats(&self) -> &ImportStats {
        &self.stats
    }

    pub fn into_scene(self) -> Scene {
        self.scene
    }

    pub fn into_parts(self) -> (Scene, Vec<Diagnostic>, FormatInfo, ImportStage, ImportStats) {
        (
            self.scene,
            self.diagnostics,
            self.format,
            self.stage,
            self.stats,
        )
    }

    pub fn map_scene(
        mut self,
        stage: ImportStage,
        process: impl FnOnce(Scene) -> Result<Scene>,
    ) -> Result<Self> {
        self.scene = process(self.scene)?;
        self.stats.record_scene_counts(&self.scene)?;
        self.stage = stage;
        Ok(self)
    }
}

pub struct ImportContext<'a> {
    io: &'a dyn AssetIo,
    source: AssetPath,
    options: ImportOptions,
    diagnostics: Vec<Diagnostic>,
    ledger: ResourceLedger,
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
        if self.diagnostics.len() < self.options.limits.max_diagnostics {
            self.diagnostics.push(diagnostic);
            self.ledger.record_diagnostic(true);
        } else {
            self.ledger.record_diagnostic(false);
        }
    }

    pub fn into_scene_report(self, scene: Scene) -> (Scene, Vec<Diagnostic>) {
        (scene, self.diagnostics)
    }

    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    pub fn into_report(mut self, scene: Scene, format: FormatInfo) -> Result<ImportReport> {
        self.ledger.record_scene(&scene, &self.options.limits)?;
        if self.options.diagnostics.strict
            && let Some(diagnostic) = self
                .diagnostics
                .iter()
                .find(|diagnostic| diagnostic.severity != baozi_core::DiagnosticSeverity::Info)
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
            stage: ImportStage::ValidatedImported,
            stats: self.ledger.into_stats(),
        })
    }

    pub fn read_primary_to_end(&mut self) -> Result<Vec<u8>> {
        let source = self.source.clone();
        let bytes = self.read_asset_to_end(
            &source,
            self.options.limits.max_primary_asset_bytes,
            "max_primary_asset_bytes",
        )?;
        self.ledger
            .debit_primary_bytes(bytes.len() as u64, &self.options.limits)?;
        Ok(bytes)
    }

    pub fn read_sidecar_to_end(&mut self, path: &AssetPath) -> Result<Vec<u8>> {
        let bytes = self.read_asset_to_end(
            path,
            self.options.limits.max_sidecar_asset_bytes,
            "max_sidecar_asset_bytes",
        )?;
        self.ledger
            .debit_sidecar_bytes(bytes.len() as u64, &self.options.limits)?;
        Ok(bytes)
    }

    pub fn resolve_source_relative(&self, relative: &str) -> Result<AssetPath> {
        self.io.resolve(&self.source, relative)
    }

    pub fn resolve_relative(&self, base: &AssetPath, relative: &str) -> Result<AssetPath> {
        self.io.resolve(base, relative)
    }

    fn read_asset_to_end(
        &mut self,
        path: &AssetPath,
        limit: u64,
        limit_name: &'static str,
    ) -> Result<Vec<u8>> {
        self.ledger.debit_open_asset(&self.options.limits)?;
        let mut reader = self.io.open(path)?;
        let mut bytes = Vec::new();
        let mut limited = reader.by_ref().take(limit.saturating_add(1));
        limited
            .read_to_end(&mut bytes)
            .map_err(|error| BaoziError::io(path.to_string(), error.to_string()))?;
        if bytes.len() as u64 > limit {
            return Err(BaoziError::LimitExceeded { limit: limit_name });
        }
        Ok(bytes)
    }
}
