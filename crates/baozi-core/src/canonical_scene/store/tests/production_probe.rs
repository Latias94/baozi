mod report;
mod scenarios;

use report::{
    ProductionReport, ProductionReportScenarios, REPORT_PREFIX, hex_encode, render, sha256_hex,
};
use scenarios::{
    collect_gltf_capability, collect_inherited_hot_leaf_scenario, collect_minimal_fallback,
    collect_obj_capability, collect_pending_scene_activation, collect_profile_closure,
    collect_protected_terminal_activation, collect_scale_scenarios,
    collect_segment_partition_invariance, collect_validation_faults,
};

#[test]
#[ignore = "run via baozi-xtask u4c-metrics with the production specification"]
fn canonical_store_u4c_production_v2_probe() {
    let report = ProductionReport::new(ProductionReportScenarios {
        scale_scenarios: collect_scale_scenarios(),
        inherited_hot_leaf_scenario: collect_inherited_hot_leaf_scenario(),
        obj_capability: collect_obj_capability(),
        gltf_capability: collect_gltf_capability(),
        profile_closure: collect_profile_closure(),
        segment_partition_invariance: collect_segment_partition_invariance(),
        validation_faults: collect_validation_faults(),
        pending_scene_activation: collect_pending_scene_activation(),
        protected_terminal_activation: collect_protected_terminal_activation(),
        minimal_fallback: collect_minimal_fallback(),
    });
    println!("\n{REPORT_PREFIX}{}", render(&report));
}

#[test]
fn allocation_counter_canary_counts_box_and_vec() {
    let box_info = allocation_counter::measure(|| {
        let value = std::hint::black_box(Box::new(7_u64));
        let _ = std::hint::black_box(value);
    });
    let vec_info = allocation_counter::measure(|| {
        let value = std::hint::black_box(vec![7_u8; 64]);
        let _ = std::hint::black_box(value);
    });
    assert!(box_info.count_total > 0);
    assert!(box_info.bytes_total > 0);
    assert_eq!(box_info.count_current, 0);
    assert_eq!(box_info.bytes_current, 0);
    assert!(vec_info.count_total > 0);
    assert!(vec_info.bytes_total > 0);
    assert_eq!(vec_info.count_current, 0);
    assert_eq!(vec_info.bytes_current, 0);
}

#[test]
fn production_report_static_vectors_are_stable() {
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    );
    assert_eq!(hex_encode(&[0x00, 0xab, 0xff]), "00abff");
    assert_eq!(REPORT_PREFIX, "BAOZI_U4C_PRODUCTION_V2=");
}
