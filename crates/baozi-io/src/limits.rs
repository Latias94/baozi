#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLimits {
    pub max_primary_asset_bytes: u64,
    pub max_sidecar_asset_bytes: u64,
    pub max_total_asset_bytes: u64,
    pub max_open_assets: usize,
    pub max_include_depth: usize,
    pub max_archive_entries: usize,
    pub max_archive_uncompressed_bytes: u64,
    pub max_data_uri_bytes: u64,
    pub max_path_length: usize,
    pub max_string_bytes: usize,
    pub max_line_bytes: usize,
    pub max_token_bytes: usize,
    pub max_meshes: usize,
    pub max_vertices: usize,
    pub max_vertex_attribute_streams: usize,
    pub max_vertex_attribute_cells: usize,
    pub max_faces: usize,
    pub max_solids: usize,
    pub max_diagnostics: usize,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_primary_asset_bytes: 512 * 1024 * 1024,
            max_sidecar_asset_bytes: 128 * 1024 * 1024,
            max_total_asset_bytes: 1024 * 1024 * 1024,
            max_open_assets: 1024,
            max_include_depth: 32,
            max_archive_entries: 100_000,
            max_archive_uncompressed_bytes: 1024 * 1024 * 1024,
            max_data_uri_bytes: 64 * 1024 * 1024,
            max_path_length: 4096,
            max_string_bytes: 16 * 1024 * 1024,
            max_line_bytes: 1024 * 1024,
            max_token_bytes: 1024 * 1024,
            max_meshes: 1_000_000,
            max_vertices: 50_000_000,
            max_vertex_attribute_streams: 256,
            max_vertex_attribute_cells: 50_000_000,
            max_faces: 50_000_000,
            max_solids: 1_000_000,
            max_diagnostics: 10_000,
        }
    }
}
