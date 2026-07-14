const SCOPE_SOURCE: &str = include_str!("../src/scope.rs");
const METERED_STORAGE_SOURCE: &str = include_str!("../src/metered_storage.rs");
const IMPORT_LIB_SOURCE: &str = include_str!("../src/lib.rs");
const CORE_SCENE_SOURCE: &str = include_str!("../../baozi-core/src/scene.rs");
const CORE_SCENE_STORAGE_SOURCE: &str = include_str!("../../baozi-core/src/scene_storage.rs");
const GLTF_PARSER_SOURCE: &str = include_str!("../../baozi-format-gltf/src/parser.rs");
const OBJ_MESH_BUILDER_SOURCE: &str = include_str!("../../baozi-format-obj/src/mesh_builder.rs");
const PLY_PARSER_SOURCE: &str = include_str!("../../baozi-format-ply/src/parser.rs");
const STL_PARSER_SOURCE: &str = include_str!("../../baozi-format-stl/src/parser.rs");

#[test]
fn diagnostic_line_index_cache_uses_the_shared_accounted_owner() {
    assert!(
        SCOPE_SOURCE.contains(
            "diagnostic_line_indexes: &'session RefCell<AccountedVec<DiagnosticLineIndex>>"
        )
    );
    assert!(SCOPE_SOURCE.contains("line_starts: AccountedVec<u64>"));
    assert!(!SCOPE_SOURCE.contains("line_starts: Vec<u64>"));
    assert!(!SCOPE_SOURCE.contains("CapacityPlan::for_type::<DiagnosticLineIndex>"));
}

#[test]
fn scoped_scene_builder_couples_its_reservation_inventory_capacity() {
    assert!(SCOPE_SOURCE.contains("payload: AccountedReservationVec"));
    assert!(!SCOPE_SOURCE.contains("reservation_list_capacity:"));
    assert!(!SCOPE_SOURCE.contains("CapacityPlan::for_type::<ImportReservation>"));
}

#[test]
fn prepared_scene_values_use_one_payload_reservation_lane() {
    assert!(METERED_STORAGE_SOURCE.contains("pub(crate) payload: AccountedReservationVec"));
    assert!(!METERED_STORAGE_SOURCE.contains("node_children: AccountedReservationVec"));
    assert!(!METERED_STORAGE_SOURCE.contains("node_mesh_bindings: AccountedReservationVec"));
    assert!(!SCOPE_SOURCE.contains("pub fn reserve_node_children("));
    assert!(!SCOPE_SOURCE.contains("pub fn reserve_node_mesh_bindings("));
    assert!(!CORE_SCENE_STORAGE_SOURCE.contains("node_children_bytes:"));
    assert!(!CORE_SCENE_STORAGE_SOURCE.contains("node_mesh_binding_bytes:"));
}

#[test]
fn scene_metadata_entries_own_their_prepared_payload_reservations() {
    assert!(SCOPE_SOURCE.contains("metadata_entries: AccountedVec<SceneValueReservations>"));
    assert!(SCOPE_SOURCE.contains("pub fn insert_prepared_metadata("));
    assert!(!SCOPE_SOURCE.contains("pub fn insert_metadata("));
}

#[test]
fn scene_builder_uses_typed_replacement_ownership() {
    assert!(SCOPE_SOURCE.contains("inventory: SceneReservationInventory"));
    assert!(!SCOPE_SOURCE.contains("fn retain_capacity("));
    assert!(!SCOPE_SOURCE.contains(".apply_collection_capacity_plan("));
    assert!(!SCOPE_SOURCE.contains(".apply_node_children_capacity_plan("));
    assert!(!SCOPE_SOURCE.contains("pub fn adopt_reservation("));
    assert!(!SCOPE_SOURCE.contains("pub fn reserve_live("));
    assert!(!SCOPE_SOURCE.contains("pub fn resize_live("));
    for raw_insert in [
        "pub fn add_node(",
        "pub fn add_child_node(",
        "pub fn add_mesh(",
        "pub fn add_material(",
        "pub fn add_texture(",
        "pub fn add_animation(",
        "pub fn add_camera(",
        "pub fn add_light(",
        "pub fn add_skin(",
    ] {
        assert!(
            !SCOPE_SOURCE.contains(raw_insert),
            "scoped scene builder retained raw insertion API {raw_insert}"
        );
    }

    assert!(!METERED_STORAGE_SOURCE.contains("pub struct MeteredStorage"));
    assert!(!METERED_STORAGE_SOURCE.contains("pub struct SceneValueReservations"));
    assert!(!IMPORT_LIB_SOURCE.contains("MeteredStorage"));
    assert!(!IMPORT_LIB_SOURCE.contains("ImportLiveReservation"));
    assert!(!IMPORT_LIB_SOURCE.contains("SceneValueReservations"));

    assert!(CORE_SCENE_SOURCE.contains("pub struct PreparedSceneInsert"));
    assert!(CORE_SCENE_SOURCE.contains("pub struct PreparedNodeInsert"));
    assert!(CORE_SCENE_SOURCE.contains("pub struct PreparedNodeMeshBindingInsert"));
    assert!(CORE_SCENE_SOURCE.contains("pub struct PreparedSceneMetadataInsert"));
}

#[test]
fn first_party_formats_transfer_payload_and_tokens_as_one_prepared_value() {
    for producer_source in [
        GLTF_PARSER_SOURCE,
        OBJ_MESH_BUILDER_SOURCE,
        PLY_PARSER_SOURCE,
        STL_PARSER_SOURCE,
    ] {
        assert!(producer_source.contains("try_into_scene_parts"));
        assert!(producer_source.contains("add_prepared_"));
    }
}
