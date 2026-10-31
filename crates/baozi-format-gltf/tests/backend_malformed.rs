mod common;

use baozi_format_gltf::GltfFailureCode;
use baozi_import::ImportDimension;
use common::{
    GltfTestOptions, TestResult, assert_gltf_failure, expected_error, glb_with_json,
    import_assets_result,
};
use serde_json::{Value, json};

const POSITION_OOB: &[u8] = include_bytes!("fixtures/backend_malformed/position_oob.gltf");
const PERSPECTIVE_WITHOUT_PAYLOAD: &[u8] =
    include_bytes!("fixtures/backend_malformed/perspective_without_payload.gltf");
const ORTHOGRAPHIC_WITHOUT_PAYLOAD: &[u8] =
    include_bytes!("fixtures/backend_malformed/orthographic_without_payload.gltf");

// Call the backend directly: the importer panic fallback must not make these regressions pass.
fn assert_backend_rejection(document: &[u8]) {
    let root = gltf::json::Root::from_slice(document).expect("fixture must deserialize");
    let error =
        gltf::Document::from_json(root).expect_err("backend must reject malformed metadata");
    assert!(
        matches!(error, gltf::Error::Validation(_)),
        "expected a backend validation error, got {error:?}"
    );
}

fn assert_import_rejection(source: &'static str, document: Vec<u8>) -> TestResult<()> {
    let (result, diagnostics) =
        import_assets_result(source, [(source, document)], GltfTestOptions::memory())?;
    let failure = expected_error(result)?;
    assert!(diagnostics.is_empty());
    assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    for dimension in [
        ImportDimension::DependencyReference,
        ImportDimension::DependencyBytes,
        ImportDimension::CanonicalNodes,
        ImportDimension::AnimationCells,
    ] {
        assert_eq!(
            failure
                .ledger()
                .dimension(dimension)
                .map_or(0, |row| row.consumed()),
            0,
            "malformed metadata must fail before {dimension:?}"
        );
    }
    Ok(())
}

#[test]
fn backend_position_reference_returns_an_error_without_unwinding() {
    assert_backend_rejection(POSITION_OOB);
}

#[test]
fn position_reference_is_rejected_without_panic_fallback_in_json_and_glb() -> TestResult<()> {
    assert_import_rejection("position-oob.gltf", POSITION_OOB.to_vec())?;
    assert_import_rejection("position-oob.glb", glb_with_json(POSITION_OOB))?;
    Ok(())
}

#[test]
fn backend_camera_kind_requires_its_projection_payload() {
    assert_backend_rejection(PERSPECTIVE_WITHOUT_PAYLOAD);
    assert_backend_rejection(ORTHOGRAPHIC_WITHOUT_PAYLOAD);
}

#[test]
fn camera_kind_is_rejected_before_projection_reading_in_json_and_glb() -> TestResult<()> {
    for document in [PERSPECTIVE_WITHOUT_PAYLOAD, ORTHOGRAPHIC_WITHOUT_PAYLOAD] {
        assert_import_rejection("camera-kind.gltf", document.to_vec())?;
        assert_import_rejection("camera-kind.glb", glb_with_json(document))?;
    }
    Ok(())
}

#[test]
fn valid_camera_discriminators_remain_readable() -> TestResult<()> {
    for (document, kind) in [
        (PERSPECTIVE_WITHOUT_PAYLOAD, "orthographic"),
        (ORTHOGRAPHIC_WITHOUT_PAYLOAD, "perspective"),
    ] {
        let mut root: Value = serde_json::from_slice(document)?;
        root["cameras"][0]["type"] = json!(kind);
        let document = gltf::Gltf::from_slice(&serde_json::to_vec(&root)?)?.document;
        let camera = document.cameras().next().expect("fixture has one camera");
        assert!(matches!(
            (kind, camera.projection()),
            ("orthographic", gltf::camera::Projection::Orthographic(_))
                | ("perspective", gltf::camera::Projection::Perspective(_))
        ));
    }
    Ok(())
}

fn animation_metadata() -> Value {
    json!({
        "asset": { "version": "2.0" },
        "nodes": [{}],
        "buffers": [{ "uri": "unopened.bin", "byteLength": 32 }],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": 8 },
            { "buffer": 0, "byteOffset": 8, "byteLength": 24 }
        ],
        "accessors": [
            {
                "bufferView": 0, "componentType": 5126, "count": 2,
                "type": "SCALAR", "min": [0], "max": [1]
            },
            { "bufferView": 1, "componentType": 5126, "count": 2, "type": "VEC3" }
        ],
        "animations": [{
            "samplers": [{ "input": 0, "output": 1, "interpolation": "LINEAR" }],
            "channels": [{ "sampler": 0, "target": { "node": 0, "path": "translation" } }]
        }]
    })
}

#[test]
fn invalid_animation_sampler_metadata_is_rejected_before_reading() -> TestResult<()> {
    let baseline = animation_metadata();
    gltf::Gltf::from_slice(&serde_json::to_vec(&baseline)?)?;

    let mut missing_sampler = baseline.clone();
    missing_sampler["animations"][0]["channels"][0]["sampler"] = json!(1);
    let mut invalid_interpolation = baseline.clone();
    invalid_interpolation["animations"][0]["samplers"][0]["interpolation"] = json!("INVALID");
    let mut missing_input = baseline.clone();
    missing_input["animations"][0]["samplers"][0]["input"] = json!(2);
    let mut missing_output = baseline;
    missing_output["animations"][0]["samplers"][0]["output"] = json!(2);

    for root in [
        missing_sampler,
        invalid_interpolation,
        missing_input,
        missing_output,
    ] {
        let document = serde_json::to_vec(&root)?;
        assert_backend_rejection(&document);
        assert_import_rejection("invalid-sampler.gltf", document)?;
    }
    Ok(())
}

#[test]
fn animation_target_checks_precede_backend_reader_calls() -> TestResult<()> {
    let baseline = animation_metadata();
    let mut missing_node = baseline.clone();
    missing_node["animations"][0]["channels"][0]["target"]["node"] = json!(1);
    let mut invalid_path = baseline;
    invalid_path["animations"][0]["channels"][0]["target"]["path"] = json!("INVALID");

    for root in [missing_node, invalid_path] {
        assert_import_rejection("invalid-target.gltf", serde_json::to_vec(&root)?)?;
    }
    Ok(())
}
