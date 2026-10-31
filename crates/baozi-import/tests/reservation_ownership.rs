const SCOPE_SOURCE: &str = include_str!("../src/scope.rs");

#[test]
fn diagnostics_do_not_rebuild_line_indexes_from_arbitrary_source_bytes() {
    assert!(!SCOPE_SOURCE.contains("DiagnosticLineIndex"));
    assert!(!SCOPE_SOURCE.contains("diagnostic-line-index"));
    assert!(!SCOPE_SOURCE.contains("pub fn emit_diagnostic_with_location("));
    assert!(SCOPE_SOURCE.contains("pub fn emit_text_diagnostic_with("));
}
