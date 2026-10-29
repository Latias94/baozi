mod common;

use baozi_import::{
    ImportCause, ImportDiagnostic, ImportDimension, ImportReport, ImportResult,
    ManifestResourceIdentityStability, SceneSnapshotLimits, ScopeAbortCause,
    SourceScopeDisposition, SourceScopeEntityKind,
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

fn split_buffer_provenance_fixture() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let buffer = triangle_bin();
    let attributes = buffer[..96].to_vec();
    let indices = buffer[96..102].to_vec();
    let source = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(
            r#"  "buffers": [{ "uri": "triangle.bin", "byteLength": 104 }],"#,
            r#"  "buffers": [{ "uri": "indices.bin", "byteLength": 6 }, { "uri": "attributes.bin", "byteLength": 96 }],"#,
        )
        .replace(
            r#"{ "buffer": 0, "byteOffset": 0, "byteLength": 36, "target": 34962 }"#,
            r#"{ "buffer": 1, "byteOffset": 0, "byteLength": 36, "target": 34962 }"#,
        )
        .replace(
            r#"{ "buffer": 0, "byteOffset": 36, "byteLength": 36, "target": 34962 }"#,
            r#"{ "buffer": 1, "byteOffset": 36, "byteLength": 36, "target": 34962 }"#,
        )
        .replace(
            r#"{ "buffer": 0, "byteOffset": 72, "byteLength": 24, "target": 34962 }"#,
            r#"{ "buffer": 1, "byteOffset": 72, "byteLength": 24, "target": 34962 }"#,
        )
        .replace(
            r#"{ "buffer": 0, "byteOffset": 96, "byteLength": 6, "target": 34963 }"#,
            r#"{ "buffer": 0, "byteOffset": 0, "byteLength": 6, "target": 34963 }"#,
        )
        .into_bytes();
    (source, indices, attributes)
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

    let manifest = report.successful_input_manifest();
    assert_eq!(manifest.counts().node_class_count(), 3);
    assert_eq!(manifest.counts().quotient_relation_record_count(), 2);
    assert_eq!(manifest.counts().relation_occurrence_count(), 2);
    assert_eq!(manifest.counts().available_relation_occurrence_count(), 2);
    assert_eq!(manifest.counts().maximum_depth(), 1);
    let mut identities = manifest
        .node_classes()
        .iter()
        .map(|class| class.identity().value().as_str())
        .collect::<Vec<_>>();
    identities.sort_unstable();
    assert_eq!(
        identities,
        [
            "models/scene.gltf",
            "models/textures/base.png",
            "models/triangle.bin",
        ]
    );
    assert!(
        manifest
            .node_classes()
            .iter()
            .all(|class| { !class.identity().value().as_str().contains("omitted") })
    );
    let mut roles = manifest
        .node_classes()
        .iter()
        .flat_map(|class| class.outgoing_relations())
        .map(|relation| relation.role().as_str())
        .collect::<Vec<_>>();
    roles.sort_unstable();
    assert_eq!(roles, ["external-buffer", "external-image"]);
    for identity in ["models/triangle.bin", "models/textures/base.png"] {
        let dependency = manifest
            .node_classes()
            .iter()
            .find(|class| class.identity().value().as_str() == identity)
            .expect("selected dependency must enter the manifest");
        assert!(matches!(
            dependency.identity().stability(),
            ManifestResourceIdentityStability::Stable { .. }
        ));
    }

    let snapshot = report
        .scene()
        .capture_snapshot(SceneSnapshotLimits::try_new(32 * 1024, 256, 1_000_000)?)?;
    assert_eq!(snapshot.profile_resource_count(), 3);
    let expected_selected_inventory = [
        (SourceScopeEntityKind::Scene, 1),
        (SourceScopeEntityKind::Node, 2),
        (SourceScopeEntityKind::Mesh, 1),
        (SourceScopeEntityKind::Primitive, 1),
        (SourceScopeEntityKind::Material, 1),
        (SourceScopeEntityKind::Texture, 1),
        (SourceScopeEntityKind::Image, 1),
        (SourceScopeEntityKind::Sampler, 1),
        (SourceScopeEntityKind::Skin, 0),
        (SourceScopeEntityKind::Camera, 0),
        (SourceScopeEntityKind::Animation, 0),
        (SourceScopeEntityKind::AnimationChannel, 0),
        (SourceScopeEntityKind::AnimationSampler, 0),
        (SourceScopeEntityKind::Buffer, 1),
        (SourceScopeEntityKind::BufferView, 4),
        (SourceScopeEntityKind::Accessor, 4),
    ];
    let selected_source_count: usize = expected_selected_inventory
        .iter()
        .map(|(_, count)| count)
        .sum();
    for (kind, expected) in expected_selected_inventory {
        assert_eq!(
            record.counts(kind).selected(),
            u64::try_from(expected)?,
            "selected inventory changed for {kind:?}",
        );
    }
    // Document reuses resource leaf zero; only the selected declarations add source leaves.
    let expected_source_count = snapshot.profile_resource_count() + selected_source_count;
    assert_eq!(snapshot.profile_source_count(), expected_source_count);
    assert_eq!(
        snapshot.profile_provenance_node_count(),
        expected_source_count + 1,
    );
    assert!(snapshot.profile_attachment_count() > snapshot.profile_owner_count());
    Ok(())
}

#[test]
fn acquired_external_buffer_tail_changes_manifest_without_changing_scene() -> TestResult<()> {
    let baseline_buffer = triangle_bin();
    let mut extended_buffer = baseline_buffer.clone();
    extended_buffer.extend_from_slice(b"manifest-only-tail");

    let (baseline, baseline_diagnostics) = import_assets(
        "models/scene.gltf",
        [
            ("models/scene.gltf", triangle_gltf()),
            ("models/triangle.bin", baseline_buffer),
            ("models/textures/base.png", triangle_png()),
        ],
        sidecar_options(),
    )?;
    let (extended, extended_diagnostics) = import_assets(
        "models/scene.gltf",
        [
            ("models/scene.gltf", triangle_gltf()),
            ("models/triangle.bin", extended_buffer.clone()),
            ("models/textures/base.png", triangle_png()),
        ],
        sidecar_options(),
    )?;

    assert!(baseline_diagnostics.is_empty());
    assert!(extended_diagnostics.is_empty());
    let limits = SceneSnapshotLimits::try_new(32 * 1024, 256, 1_000_000)?;
    let baseline_scene = baseline.scene().capture_snapshot(limits)?;
    let extended_scene = extended.scene().capture_snapshot(limits)?;
    assert_eq!(baseline_scene.digest(), extended_scene.digest());
    assert_ne!(
        baseline.successful_input_manifest().digest(),
        extended.successful_input_manifest().digest()
    );
    let extended_node = extended
        .successful_input_manifest()
        .node_classes()
        .iter()
        .find(|class| class.identity().value().as_str() == "models/triangle.bin")
        .expect("external buffer must enter the manifest");
    assert_eq!(extended_node.byte_length(), extended_buffer.len() as u64);
    Ok(())
}

#[test]
fn multiple_buffer_provenance_is_canonicalized_by_source_id() -> TestResult<()> {
    let (document, indices, attributes) = split_buffer_provenance_fixture();
    let (report, diagnostics) = import_assets(
        "models/split-buffer.gltf",
        [
            ("models/split-buffer.gltf", document),
            ("models/indices.bin", indices),
            ("models/attributes.bin", attributes),
            ("models/textures/base.png", triangle_png()),
        ],
        sidecar_options(),
    )?;

    assert!(diagnostics.is_empty());
    assert_eq!(
        report
            .successful_input_manifest()
            .counts()
            .node_class_count(),
        4
    );
    let snapshot = report
        .scene()
        .capture_snapshot(SceneSnapshotLimits::try_new(32 * 1024, 256, 1_000_000)?)?;
    assert_eq!(snapshot.profile_resource_count(), 4);
    assert!(snapshot.profile_attachment_count() > snapshot.profile_owner_count());
    Ok(())
}

#[test]
fn resource_closure_work_budget_fails_before_scope_or_dependency_io() -> TestResult<()> {
    let document = selected_resource_closure_fixture();
    let mut options = sidecar_options();
    // The immutable primary hash completes first. No subsequent semantic preflight work can
    // start after this exact budget, regardless of later URI or source-scope work additions.
    options.limits.max_work_units = document.len() as u64;
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
    assert_eq!(snapshot.profile_resource_count(), 2);
    let record = report
        .source_scope_record()
        .expect("buffer-view image must retain source scope evidence");
    let expected_selected_inventory = [
        (SourceScopeEntityKind::Scene, 1),
        (SourceScopeEntityKind::Node, 2),
        (SourceScopeEntityKind::Mesh, 1),
        (SourceScopeEntityKind::Primitive, 1),
        (SourceScopeEntityKind::Material, 1),
        (SourceScopeEntityKind::Texture, 1),
        (SourceScopeEntityKind::Image, 1),
        (SourceScopeEntityKind::Sampler, 1),
        (SourceScopeEntityKind::Skin, 0),
        (SourceScopeEntityKind::Camera, 0),
        (SourceScopeEntityKind::Animation, 0),
        (SourceScopeEntityKind::AnimationChannel, 0),
        (SourceScopeEntityKind::AnimationSampler, 0),
        (SourceScopeEntityKind::Buffer, 1),
        // Four geometry views and the additional encoded-image view are all selected.
        (SourceScopeEntityKind::BufferView, 5),
        (SourceScopeEntityKind::Accessor, 4),
    ];
    let selected_source_count: usize = expected_selected_inventory
        .iter()
        .map(|(_, count)| count)
        .sum();
    for (kind, expected) in expected_selected_inventory {
        assert_eq!(
            record.counts(kind).selected(),
            u64::try_from(expected)?,
            "selected inventory changed for {kind:?}",
        );
    }
    // The image has a source declaration but shares the selected buffer's resource leaf.
    let expected_source_count = snapshot.profile_resource_count() + selected_source_count;
    assert_eq!(snapshot.profile_source_count(), expected_source_count);
    assert_eq!(
        snapshot.profile_provenance_node_count(),
        expected_source_count + 1,
    );
    assert!(snapshot.profile_attachment_count() > snapshot.profile_owner_count());
    let manifest = report.successful_input_manifest();
    assert_eq!(manifest.counts().node_class_count(), 2);
    assert_eq!(manifest.counts().relation_occurrence_count(), 1);
    let relation = manifest
        .node_classes()
        .iter()
        .flat_map(|class| class.outgoing_relations())
        .next()
        .expect("buffer-view image must retain its external buffer relation");
    assert_eq!(relation.role().as_str(), "external-buffer");
    Ok(())
}
