use baozi::{
    ConformancePolicy, ImportConfigOverlay, ImportPreset, ImportRequest, Importer,
    OptionCompileLimits,
};

#[test]
fn facade_prepares_a_reusable_import_preset_without_lower_crate_types() {
    let preset = ImportPreset::new(
        ImportConfigOverlay::default()
            .with_conformance_policy(ConformancePolicy::AllowDeclaredRecovery),
    );
    let request = ImportRequest::new()
        .try_with_preset(&preset)
        .expect("facade preset expansion");
    let importer = Importer::empty().expect("empty facade registry");

    let prepared = importer
        .prepare_request(&request, OptionCompileLimits::default())
        .expect("facade request preparation");

    assert_eq!(
        prepared.config().conformance_policy(),
        ConformancePolicy::AllowDeclaredRecovery
    );
}
