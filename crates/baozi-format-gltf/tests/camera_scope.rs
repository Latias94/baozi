use baozi_import::{SourceScopeDisposition, SourceScopeEntityKind};
use common::{GltfTestOptions, TestResult, import_assets};

mod common;

#[test]
fn source_scope_records_selected_and_omitted_cameras() -> TestResult<()> {
    let document = br#"{
  "asset": { "version": "2.0", "generator": "baozi-camera-scope-test" },
  "scene": 0,
  "scenes": [{ "nodes": [0] }],
  "nodes": [{ "name": "SelectedCameraNode", "camera": 0 }, { "name": "OmittedCameraNode", "camera": 1 }],
  "cameras": [
    { "name": "Main", "type": "perspective", "perspective": { "yfov": 1.0, "znear": 0.1, "zfar": 100.0 } },
    { "name": "Omitted", "type": "orthographic", "orthographic": { "xmag": 1.0, "ymag": 1.0, "znear": 0.1, "zfar": 100.0 } }
  ]
}"#
    .to_vec();
    let (report, diagnostics) = import_assets(
        "models/cameras.gltf",
        [("models/cameras.gltf", document)],
        GltfTestOptions::memory(),
    )?;

    assert!(diagnostics.is_empty());
    let record = report
        .source_scope_record()
        .expect("camera projection must publish source-scope evidence");
    assert_eq!(record.counts(SourceScopeEntityKind::Camera).selected(), 1);
    assert_eq!(record.counts(SourceScopeEntityKind::Camera).omitted(), 1);
    assert!(record.details().any(|detail| {
        detail.kind() == SourceScopeEntityKind::Camera
            && detail.disposition() == SourceScopeDisposition::Selected
            && detail.identity() == "c0"
    }));
    assert!(record.details().any(|detail| {
        detail.kind() == SourceScopeEntityKind::Camera
            && detail.disposition() == SourceScopeDisposition::Omitted
            && detail.identity() == "c1"
    }));
    Ok(())
}
