use crate::descriptor::TextProfile;

/// Frozen resource bounds for one import session.
///
/// These limits cover import-owned aggregate work. Backend hard limits and
/// format descriptor limits may only lower the effective value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResourceLimits {
    pub max_primary_asset_bytes: u64,
    pub max_dependency_asset_bytes: u64,
    pub max_total_asset_bytes: u64,
    pub max_open_assets: u64,
    pub max_dependency_references: u64,
    pub max_include_depth: usize,
    pub max_data_uri_bytes: u64,
    pub max_live_bytes: u64,
    pub max_work_units: u64,
    pub max_detection_work: u64,
    pub max_decode_work: u64,
    pub max_decoded_utf8_bytes: u64,
    pub max_line_index_bytes: u64,
    pub max_tokens: u64,
    pub max_path_length: usize,
    pub max_string_bytes: usize,
    pub max_canonical_string_bytes: usize,
    pub max_line_bytes: usize,
    pub max_token_bytes: usize,
    pub max_records: usize,
    pub max_meshes: usize,
    pub max_primitives: usize,
    pub max_vertices: usize,
    pub max_vertex_attribute_streams_per_mesh: usize,
    pub max_total_vertex_attribute_streams: usize,
    pub max_total_vertex_attribute_cells: usize,
    pub max_total_topology_cells: usize,
    pub max_faces: usize,
    pub max_influence_cells: usize,
    pub max_morph_cells: usize,
    pub max_animation_cells: usize,
    pub max_canonical_payload_bytes: u64,
    pub max_solids: usize,
    pub max_graph_nodes: usize,
    pub max_graph_edges: usize,
    pub max_graph_depth: usize,
    pub max_graph_work: usize,
    pub max_diagnostics: u64,
    pub max_attempted_diagnostics: u64,
    pub max_diagnostic_bytes: u64,
    pub max_diagnostic_message_bytes: usize,
    pub max_attempted_diagnostic_bytes: u64,
    pub max_loss_evidence: u64,
    pub max_attempted_recoveries: u64,
    pub max_loss_evidence_bytes: u64,
    pub max_attempted_recovery_bytes: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_primary_asset_bytes: 512 * 1024 * 1024,
            max_dependency_asset_bytes: 128 * 1024 * 1024,
            max_total_asset_bytes: 1024 * 1024 * 1024,
            max_open_assets: 1024,
            max_dependency_references: 16_384,
            max_include_depth: 32,
            max_data_uri_bytes: 64 * 1024 * 1024,
            max_live_bytes: 1536 * 1024 * 1024,
            max_work_units: 200_000_000,
            max_detection_work: 4_000_000,
            max_decode_work: 200_000_000,
            max_decoded_utf8_bytes: 1024 * 1024 * 1024,
            max_line_index_bytes: 64 * 1024 * 1024,
            max_tokens: 1_000_000_000,
            max_path_length: 4096,
            max_string_bytes: 16 * 1024 * 1024,
            max_canonical_string_bytes: 64 * 1024 * 1024,
            max_line_bytes: 1024 * 1024,
            max_token_bytes: 1024 * 1024,
            max_records: 100_000_000,
            max_meshes: 1_000_000,
            max_primitives: 16_000_000,
            max_vertices: 50_000_000,
            max_vertex_attribute_streams_per_mesh: 256,
            max_total_vertex_attribute_streams: 1_000_000,
            max_total_vertex_attribute_cells: 50_000_000,
            max_total_topology_cells: 50_000_000,
            max_faces: 50_000_000,
            max_influence_cells: 400_000_000,
            max_morph_cells: 400_000_000,
            max_animation_cells: 400_000_000,
            max_canonical_payload_bytes: 1024 * 1024 * 1024,
            max_solids: 1_000_000,
            max_graph_nodes: 1_000_000,
            max_graph_edges: 4_000_000,
            max_graph_depth: 1_024,
            max_graph_work: 5_000_000,
            max_diagnostics: 10_000,
            max_attempted_diagnostics: 100_000,
            max_diagnostic_bytes: 4 * 1024 * 1024,
            max_diagnostic_message_bytes: 16 * 1024,
            max_attempted_diagnostic_bytes: 16 * 1024 * 1024,
            max_loss_evidence: 10_000,
            max_attempted_recoveries: 100_000,
            max_loss_evidence_bytes: 4 * 1024 * 1024,
            max_attempted_recovery_bytes: 16 * 1024 * 1024,
        }
    }
}

macro_rules! define_resource_limits_overlay {
    ($( $field:ident: $type:ty ),+ $(,)?) => {
        /// Per-field resource-limit overrides.
        ///
        /// `None` means that the lower-precedence value is retained. An explicit
        /// value remains an override even when it equals the built-in default.
        #[derive(Debug, Clone, Default, PartialEq, Eq)]
        #[non_exhaustive]
        pub struct ResourceLimitsOverlay {
            $(pub $field: Option<$type>,)+
        }

        impl ResourceLimitsOverlay {
            pub fn from_limits(limits: ResourceLimits) -> Self {
                Self {
                    $($field: Some(limits.$field),)+
                }
            }

            pub const fn is_empty(&self) -> bool {
                $(self.$field.is_none())&&+
            }

            pub fn apply_to(&self, limits: &mut ResourceLimits) {
                $(
                    if let Some(value) = self.$field {
                        limits.$field = value;
                    }
                )+
            }
        }
    };
}

define_resource_limits_overlay! {
    max_primary_asset_bytes: u64,
    max_dependency_asset_bytes: u64,
    max_total_asset_bytes: u64,
    max_open_assets: u64,
    max_dependency_references: u64,
    max_include_depth: usize,
    max_data_uri_bytes: u64,
    max_live_bytes: u64,
    max_work_units: u64,
    max_detection_work: u64,
    max_decode_work: u64,
    max_decoded_utf8_bytes: u64,
    max_line_index_bytes: u64,
    max_tokens: u64,
    max_path_length: usize,
    max_string_bytes: usize,
    max_canonical_string_bytes: usize,
    max_line_bytes: usize,
    max_token_bytes: usize,
    max_records: usize,
    max_meshes: usize,
    max_primitives: usize,
    max_vertices: usize,
    max_vertex_attribute_streams_per_mesh: usize,
    max_total_vertex_attribute_streams: usize,
    max_total_vertex_attribute_cells: usize,
    max_total_topology_cells: usize,
    max_faces: usize,
    max_influence_cells: usize,
    max_morph_cells: usize,
    max_animation_cells: usize,
    max_canonical_payload_bytes: u64,
    max_solids: usize,
    max_graph_nodes: usize,
    max_graph_edges: usize,
    max_graph_depth: usize,
    max_graph_work: usize,
    max_diagnostics: u64,
    max_attempted_diagnostics: u64,
    max_diagnostic_bytes: u64,
    max_diagnostic_message_bytes: usize,
    max_attempted_diagnostic_bytes: u64,
    max_loss_evidence: u64,
    max_attempted_recoveries: u64,
    max_loss_evidence_bytes: u64,
    max_attempted_recovery_bytes: u64,
}

impl ResourceLimits {
    pub fn apply_overlay(&mut self, overlay: &ResourceLimitsOverlay) {
        overlay.apply_to(self);
    }

    pub fn with_overlay(mut self, overlay: &ResourceLimitsOverlay) -> Self {
        self.apply_overlay(overlay);
        self
    }

    pub(crate) fn effective_for_selected_format(
        &self,
        text_profile: Option<&TextProfile>,
        primary_backend_hard_limit: Option<u64>,
    ) -> Self {
        let mut effective = self.clone();
        if let Some(profile) = text_profile {
            let hard_caps = profile.hard_caps();
            effective.max_primary_asset_bytes = effective
                .max_primary_asset_bytes
                .min(hard_caps.max_raw_bytes());
            effective.max_dependency_asset_bytes = effective
                .max_dependency_asset_bytes
                .min(hard_caps.max_raw_bytes());
            effective.max_decoded_utf8_bytes = effective
                .max_decoded_utf8_bytes
                .min(hard_caps.max_decoded_utf8_bytes());
            effective.max_line_index_bytes = effective
                .max_line_index_bytes
                .min(hard_caps.max_line_index_bytes());
            effective.max_tokens = effective.max_tokens.min(hard_caps.max_tokens());
            effective.max_canonical_string_bytes = effective
                .max_canonical_string_bytes
                .min(usize::try_from(hard_caps.max_canonical_string_bytes()).unwrap_or(usize::MAX));
        }
        if let Some(hard_limit) = primary_backend_hard_limit {
            effective.max_primary_asset_bytes = effective.max_primary_asset_bytes.min(hard_limit);
        }
        effective
    }
}
