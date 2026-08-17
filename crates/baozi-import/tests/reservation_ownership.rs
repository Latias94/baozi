const SCOPE_SOURCE: &str = include_str!("../src/scope.rs");

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
