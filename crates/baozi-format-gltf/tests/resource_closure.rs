mod common;

use baozi_import::{
    ImportCause, ImportDiagnostic, ImportDimension, ImportReport, ImportResult,
    SceneSnapshotLimits, ScopeAbortCause, SourceScopeDisposition, SourceScopeEntityKind,
};
use baozi_operation::{CancellationOrigin, ControlCause};
use base64::Engine as _;
use common::{
    TestResult, controlled_import, expected_error, import_assets, import_assets_result,
    import_assets_result_with_control, sidecar_options, triangle_bin, triangle_gltf, triangle_png,
};
use std::sync::atomic::Ordering;

fn controlled_resource_closure_result(
    document: &[u8],
    cancel_on: Option<usize>,
) -> TestResult<(ImportResult<ImportReport>, Vec<ImportDiagnostic>, usize)> {
    let (control, polls) = controlled_import(cancel_on);
    let (result, diagnostics) = import_assets_result_with_control(
        "models/resource-closure-cancel.gltf",
        [
            ("models/resource-closure-cancel.gltf", document.to_vec()),
            ("models/triangle.bin", triangle_bin()),
            ("models/textures/base.png", triangle_png()),
        ],
        sidecar_options(),
        control,
    )?;
    Ok((result, diagnostics, polls.load(Ordering::SeqCst)))
}

fn replace_once(source: String, from: &str, to: &str) -> String {
    assert!(source.contains(from), "fixture fragment not found: {from}");
    source.replacen(from, to, 1)
}

fn data_uri_image_fixture() -> Vec<u8> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(triangle_png());
    String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(
            r#""uri": "textures/base.png""#,
            &format!(r#""uri": "data:image/png;base64,{encoded}""#),
        )
        .into_bytes()
}

fn buffer_view_image_fixture() -> (Vec<u8>, Vec<u8>) {
    let mut buffer = triangle_bin();
    let image = triangle_png();
    let image_offset = buffer.len();
    buffer.extend_from_slice(&image);
    let source = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(
            r#""byteLength": 104"#,
            &format!(r#""byteLength": {}"#, buffer.len()),
        )
        .replace(
            r#"    { "buffer": 0, "byteOffset": 96, "byteLength": 6, "target": 34963 }
  ],"#,
            &format!(
                "    {{ \"buffer\": 0, \"byteOffset\": 96, \"byteLength\": 6, \"target\": 34963 }},\n    {{ \"buffer\": 0, \"byteOffset\": {image_offset}, \"byteLength\": {} }}\n  ],",
                image.len()
            ),
        )
        .replace(
            r#"{ "uri": "textures/base.png", "name": "BaseImage" }"#,
            r#"{ "bufferView": 4, "mimeType": "image/png", "name": "BaseImage" }"#,
        )
        .into_bytes();
    (source, buffer)
}

fn selected_resource_closure_fixture() -> Vec<u8> {
    let source = String::from_utf8(triangle_gltf()).expect("fixture is valid utf-8");
    let source = replace_once(
        source,
        "  \"scenes\": [{ \"nodes\": [0] }],",
        "  \"scenes\": [{ \"nodes\": [0] }, { \"nodes\": [2] }],",
    );
    let source = replace_once(
        source,
        "    { \"name\": \"TriangleNode\", \"mesh\": 0 }",
        "    { \"name\": \"TriangleNode\", \"mesh\": 0 },\n    { \"name\": \"OmittedNode\", \"mesh\": 1 }",
    );
    let source = replace_once(
        source,
        "  \"buffers\": [{ \"uri\": \"triangle.bin\", \"byteLength\": 104 }],",
        "  \"buffers\": [{ \"uri\": \"triangle.bin\", \"byteLength\": 104 }, { \"uri\": \"omitted.bin\", \"byteLength\": 36 }],",
    );
    let source = replace_once(
        source,
        "    { \"buffer\": 0, \"byteOffset\": 96, \"byteLength\": 6, \"target\": 34963 }\n  ],",
        "    { \"buffer\": 0, \"byteOffset\": 96, \"byteLength\": 6, \"target\": 34963 },\n    { \"buffer\": 1, \"byteOffset\": 0, \"byteLength\": 36, \"target\": 34962 }\n  ],",
    );
    let source = replace_once(
        source,
        "    { \"bufferView\": 3, \"componentType\": 5123, \"count\": 3, \"type\": \"SCALAR\" }\n  ],",
        "    { \"bufferView\": 3, \"componentType\": 5123, \"count\": 3, \"type\": \"SCALAR\" },\n    { \"bufferView\": 4, \"componentType\": 5126, \"count\": 3, \"type\": \"VEC3\", \"min\": [0, 0, 0], \"max\": [1, 1, 0] }\n  ],",
    );
    let source = replace_once(
        source,
        "  \"images\": [{ \"uri\": \"textures/base.png\", \"name\": \"BaseImage\" }],",
        "  \"images\": [{ \"uri\": \"textures/base.png\", \"name\": \"BaseImage\" }, { \"uri\": \"textures/omitted.png\", \"name\": \"OmittedImage\" }],",
    );
    let source = replace_once(
        source,
        "  \"samplers\": [{ \"wrapS\": 33071, \"wrapT\": 33648, \"magFilter\": 9729, \"minFilter\": 9987 }],",
        "  \"samplers\": [{ \"wrapS\": 33071, \"wrapT\": 33648, \"magFilter\": 9729, \"minFilter\": 9987 }, { \"wrapS\": 10497, \"wrapT\": 10497 }],",
    );
    let source = replace_once(
        source,
        "  \"textures\": [{ \"source\": 0, \"sampler\": 0, \"name\": \"BaseTex\" }],",
        "  \"textures\": [{ \"source\": 0, \"sampler\": 0, \"name\": \"BaseTex\" }, { \"source\": 1, \"sampler\": 1, \"name\": \"OmittedTex\" }],",
    );
    let source = replace_once(
        source,
        "    \"doubleSided\": true\n  }]",
        "    \"doubleSided\": true\n  }, {\n    \"name\": \"OmittedMaterial\",\n    \"pbrMetallicRoughness\": { \"baseColorTexture\": { \"index\": 1 } }\n  }]",
    );
    replace_once(
        source,
        "  }]\n}",
        "  }, {\n    \"name\": \"OmittedMesh\",\n    \"primitives\": [{ \"attributes\": { \"POSITION\": 4 }, \"indices\": 3, \"material\": 1, \"mode\": 4 }]\n  }]\n}",
    )
    .into_bytes()
}

#[test]
fn selected_scene_does_not_acquire_omitted_resource_closure() -> TestResult<()> {
    let (report, diagnostics) = import_assets(
        "models/scene.gltf",
        [
            ("models/scene.gltf", selected_resource_closure_fixture()),
            ("models/triangle.bin", triangle_bin()),
            ("models/textures/base.png", triangle_png()),
        ],
        sidecar_options(),
    )?;

    assert!(diagnostics.is_empty());
    let record = report
        .source_scope_record()
        .expect("resource closure must publish source scope evidence");
    assert_eq!(record.counts(SourceScopeEntityKind::Mesh).selected(), 1);
    assert_eq!(record.counts(SourceScopeEntityKind::Mesh).omitted(), 1);
    assert_eq!(
        record.counts(SourceScopeEntityKind::Primitive).selected(),
        1
    );
    assert_eq!(record.counts(SourceScopeEntityKind::Primitive).omitted(), 1);
    assert_eq!(record.counts(SourceScopeEntityKind::Material).selected(), 1);
    assert_eq!(record.counts(SourceScopeEntityKind::Material).omitted(), 1);
    assert_eq!(record.counts(SourceScopeEntityKind::Texture).selected(), 1);
    assert_eq!(record.counts(SourceScopeEntityKind::Texture).omitted(), 1);
    assert_eq!(record.counts(SourceScopeEntityKind::Image).selected(), 1);
    assert_eq!(record.counts(SourceScopeEntityKind::Image).omitted(), 1);
    assert_eq!(record.counts(SourceScopeEntityKind::Sampler).selected(), 1);
    assert_eq!(record.counts(SourceScopeEntityKind::Sampler).omitted(), 1);
    assert_eq!(record.counts(SourceScopeEntityKind::Buffer).selected(), 1);
    assert_eq!(record.counts(SourceScopeEntityKind::Buffer).omitted(), 1);
    assert_eq!(
        record.counts(SourceScopeEntityKind::BufferView).selected(),
        4
    );
    assert_eq!(
        record.counts(SourceScopeEntityKind::BufferView).omitted(),
        1
    );
    assert_eq!(record.counts(SourceScopeEntityKind::Accessor).selected(), 4);
    assert_eq!(record.counts(SourceScopeEntityKind::Accessor).omitted(), 1);
    assert!(record.details().any(|detail| {
        detail.kind() == SourceScopeEntityKind::Buffer
            && detail.disposition() == SourceScopeDisposition::Omitted
            && detail.identity() == "b1"
    }));
    for (kind, expected) in [
        (SourceScopeEntityKind::Mesh, vec!["m0", "m1"]),
        (SourceScopeEntityKind::Primitive, vec!["m0p0", "m1p0"]),
        (SourceScopeEntityKind::Material, vec!["mat0", "mat1"]),
        (SourceScopeEntityKind::Texture, vec!["t0", "t1"]),
        (SourceScopeEntityKind::Image, vec!["i0", "i1"]),
        (SourceScopeEntityKind::Sampler, vec!["ts0", "ts1"]),
        (SourceScopeEntityKind::Buffer, vec!["b0", "b1"]),
        (
            SourceScopeEntityKind::BufferView,
            vec!["bv0", "bv1", "bv2", "bv3", "bv4"],
        ),
        (
            SourceScopeEntityKind::Accessor,
            vec!["acc0", "acc1", "acc2", "acc3", "acc4"],
        ),
    ] {
        let identities = record
            .details()
            .filter(|detail| detail.kind() == kind)
            .map(|detail| detail.identity())
            .collect::<Vec<_>>();
        assert_eq!(identities, expected, "source order changed for {kind:?}");
    }
    Ok(())
}

#[test]
fn resource_closure_work_budget_fails_before_scope_or_dependency_io() -> TestResult<()> {
    let document = selected_resource_closure_fixture();
    let (calibration, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [("models/scene.gltf", document.clone())],
        sidecar_options(),
    )?;
    let calibration_failure = expected_error(calibration)?;
    assert!(diagnostics.is_empty());
    let work_attempted = calibration_failure
        .ledger()
        .dimension(ImportDimension::WorkUnits)
        .expect("work units are configured")
        .attempted();
    assert!(work_attempted > 1);

    let mut options = sidecar_options();
    options.limits.max_work_units = work_attempted - 1;
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", document),
            ("models/triangle.bin", triangle_bin()),
            ("models/textures/base.png", triangle_png()),
        ],
        options,
    )?;
    let failure = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(abort.cause(), ScopeAbortCause::Ledger(_))
    ));
    assert!(failure.source_scope_record().is_none());
    assert_eq!(
        failure
            .ledger()
            .dimension(ImportDimension::DependencyReference)
            .expect("dependency references are configured")
            .attempted(),
        0
    );
    Ok(())
}

#[test]
fn resource_closure_cancellation_precedes_scope_publication_and_dependency_io() -> TestResult<()> {
    let document = selected_resource_closure_fixture();
    let (calibration, diagnostics, total_polls) =
        controlled_resource_closure_result(&document, None)?;
    assert!(diagnostics.is_empty());
    let _report = calibration?;
    assert!(total_polls > 0);

    let mut cancelled_before_scope = None;
    for cancel_on in 1..=total_polls {
        let (result, diagnostics, polls) =
            controlled_resource_closure_result(&document, Some(cancel_on))?;
        let failure = match result {
            Ok(_) => continue,
            Err(error) => error.into_operation_failure()?,
        };
        assert!(diagnostics.is_empty());
        assert_eq!(polls, cancel_on);
        assert!(matches!(
            failure.cause(),
            ImportCause::Scope(abort)
                if matches!(
                    abort.cause(),
                    ScopeAbortCause::Control(ControlCause::Cancelled {
                        origin: CancellationOrigin::Signal,
                    })
                )
        ));
        let dependency_references = failure
            .ledger()
            .dimension(ImportDimension::DependencyReference)
            .expect("dependency references are configured");
        let work_units = failure
            .ledger()
            .dimension(ImportDimension::WorkUnits)
            .expect("work units are configured");
        if failure.source_scope_record().is_none()
            && dependency_references.attempted() == 0
            && work_units.attempted() > 0
        {
            cancelled_before_scope = Some(failure);
            break;
        }
    }

    let failure = cancelled_before_scope
        .expect("a calibrated cancellation must stop after closure work but before scope");
    assert!(failure.source_scope_record().is_none());
    assert_eq!(
        failure
            .ledger()
            .dimension(ImportDimension::DependencyReference)
            .expect("dependency references are configured")
            .attempted(),
        0
    );
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
    Ok(())
}

#[test]
fn selected_data_uri_image_is_materialized_without_dependency_io() -> TestResult<()> {
    let (report, diagnostics) = import_assets(
        "models/scene.gltf",
        [
            ("models/scene.gltf", data_uri_image_fixture()),
            ("models/triangle.bin", triangle_bin()),
        ],
        sidecar_options(),
    )?;

    assert!(diagnostics.is_empty());
    let snapshot = report
        .scene()
        .capture_snapshot(SceneSnapshotLimits::try_new(16 * 1024, 128, 1_000_000)?)?;
    assert_eq!(snapshot.image_count(), 1);
    assert_eq!(snapshot.sampler_count(), 1);
    assert_eq!(snapshot.texture_count(), 1);
    assert_eq!(snapshot.material_binding_count(), 1);
    assert_eq!(snapshot.image_payload_bytes(), triangle_png().len() as u64);
    Ok(())
}

#[test]
fn selected_buffer_view_image_is_materialized_from_selected_buffer() -> TestResult<()> {
    let (document, buffer) = buffer_view_image_fixture();
    let (report, diagnostics) = import_assets(
        "models/scene.gltf",
        [
            ("models/scene.gltf", document),
            ("models/triangle.bin", buffer),
        ],
        sidecar_options(),
    )?;

    assert!(diagnostics.is_empty());
    let snapshot = report
        .scene()
        .capture_snapshot(SceneSnapshotLimits::try_new(16 * 1024, 128, 1_000_000)?)?;
    assert_eq!(snapshot.image_count(), 1);
    assert_eq!(snapshot.sampler_count(), 1);
    assert_eq!(snapshot.texture_count(), 1);
    assert_eq!(snapshot.material_binding_count(), 1);
    assert_eq!(snapshot.image_payload_bytes(), triangle_png().len() as u64);
    Ok(())
}
