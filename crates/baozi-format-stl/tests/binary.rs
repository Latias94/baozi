mod common;

use baozi_core::{AttributeData, AttributeDomain, TopologyKind};
use baozi_format_stl::StlFailureCode;
use baozi_import::{
    DetectionOutcome, ImportCause, ImportDimension, ImportOperationControl, ImportTerminalCause,
    ImportTerminalFailureClass, ReadConfidence, ResourceLimits,
};
use baozi_io::AssetPath;
use common::{
    BinaryFacet, TestImportOutcome, ascii_triangle, binary_stl, expected_stl_failure, import_bytes,
    import_bytes_with_limits, import_plan, scene_snapshot, snapshot_line,
};
use std::error::Error;

type TestResult = Result<(), Box<dyn Error + Send + Sync>>;

#[test]
fn planned_byte_imports_detach_from_input_and_plan() -> TestResult {
    for (encoding, bytes) in [
        (
            "binary",
            binary_stl(b"binary", &[BinaryFacet::unit_triangle()]),
        ),
        ("ascii", ascii_triangle("detached").into_bytes()),
    ] {
        let report = {
            let plan = import_plan(ResourceLimits::default(), false);
            let source = AssetPath::new(format!("detached-{encoding}.stl"))?;
            let report = TestImportOutcome::new(plan.read_bytes(
                source,
                &bytes,
                ImportOperationControl::default(),
            ))
            .expect("STL caller bytes must import without asset IO");
            drop(plan);
            drop(bytes);
            report
        };

        let snapshot = scene_snapshot(&report)?;
        assert_eq!(snapshot.primitive_index_count(), 3, "{encoding}");
        assert_eq!(snapshot.f32_scalar_count(), 18, "{encoding}");
        let mesh = report.scene().meshes().next().expect("one STL mesh");
        let primitive = mesh.primitives().next().expect("one STL primitive");
        assert_eq!(primitive.indices(), Some(&[0, 1, 2][..]), "{encoding}");
        let AttributeData::DenseF32 { lanes: 3, values } = primitive
            .normals(AttributeDomain::Vertex)
            .expect("authored facet normals")
            .data()
        else {
            panic!("expected detached dense STL normals");
        };
        assert_eq!(values, &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
        assert!(report.diagnostics().is_empty(), "{encoding}");
        assert!(report.loss_evidence().is_empty(), "{encoding}");
        assert_eq!(report.source().as_str(), format!("detached-{encoding}.stl"));
        assert_eq!(report.ledger().current_live_bytes(), 0, "{encoding}");
    }
    Ok(())
}

#[test]
fn imports_one_triangle_binary_stl_through_session() -> TestResult {
    let bytes = binary_stl(b"binary", &[BinaryFacet::unit_triangle()]);
    let report = import_bytes("triangle.stl", &bytes)?;
    let snapshot = scene_snapshot(&report)?;

    assert_eq!(snapshot.node_count(), 1);
    assert_eq!(snapshot.root_count(), 1);
    assert_eq!(snapshot.logical_mesh_count(), 1);
    assert_eq!(snapshot.primitive_count(), 1);
    assert_eq!(snapshot.attribute_count(), 2);
    assert_eq!(snapshot.primitive_index_count(), 3);
    assert_eq!(snapshot.f32_scalar_count(), 18);
    assert!(snapshot_line(&snapshot, "node[0]").contains(" name=<STL_BINARY> "));
    assert!(snapshot_line(&snapshot, "logical_mesh[0]").contains(" name=<STL_BINARY> "));
    assert!(snapshot_line(&snapshot, "attribute[0]").contains(concat!(
        "values(present,total=9)=[",
        "0x00000000,0x00000000,0x00000000,0x3f800000"
    )));
    assert!(snapshot_line(&snapshot, "attribute[1]").contains(concat!(
        "values(present,total=9)=[",
        "0x00000000,0x00000000,0x3f800000"
    )));
    assert!(report.diagnostics().is_empty());
    assert_eq!(report.ledger().current_live_bytes(), 0);
    assert!(
        report
            .ledger()
            .dimension(ImportDimension::ScratchBytes)
            .expect("STL binary scratch storage")
            .peak_live_bytes()
            > 0
    );
    assert!(
        report
            .ledger()
            .dimension(ImportDimension::CanonicalOutputBytes)
            .expect("STL binary scene storage")
            .peak_live_bytes()
            > 0
    );
    assert_eq!(
        report
            .ledger()
            .dimension(ImportDimension::DecodeWork)
            .expect("binary decode work")
            .consumed(),
        bytes.len() as u64
    );
    Ok(())
}

#[test]
fn authored_facet_payloads_are_exact_in_binary_and_ascii() -> TestResult {
    // Original facets distinguish later records, authored normals, and repeated vertices.
    let binary = binary_stl(
        b"authored facets",
        &[
            BinaryFacet {
                normal: [2.0, -3.0, 4.0],
                vertices: [[-0.0, 2.0, -3.0], [4.0, 0.5, -6.0], [-7.0, 8.0, 0.25]],
                attribute: 0,
            },
            BinaryFacet {
                normal: [-0.0, 0.0, -2.0],
                vertices: [[9.0, 10.0, 11.0], [9.0, 10.0, 11.0], [12.0, 13.0, 14.0]],
                attribute: 0,
            },
        ],
    );
    assert_eq!(binary.len(), 184);
    assert_eq!(&binary[80..84], &[2, 0, 0, 0]);
    let ascii = b"solid authored facets\n\
        facet normal 2 -3 4\nouter loop\n\
        vertex -0 2 -3\nvertex 4 0.5 -6\nvertex -7 8 0.25\n\
        endloop\nendfacet\n\
        facet normal -0 0 -2\nouter loop\n\
        vertex 9 10 11\nvertex 9 10 11\nvertex 12 13 14\n\
        endloop\nendfacet\nendsolid authored facets\n";
    let expected_positions = [
        -0.0_f32, 2.0, -3.0, 4.0, 0.5, -6.0, -7.0, 8.0, 0.25, 9.0, 10.0, 11.0, 9.0, 10.0, 11.0,
        12.0, 13.0, 14.0,
    ];
    let expected_normals = [
        2.0_f32, -3.0, 4.0, 2.0, -3.0, 4.0, 2.0, -3.0, 4.0, -0.0, 0.0, -2.0, -0.0, 0.0, -2.0, -0.0,
        0.0, -2.0,
    ];

    for (encoding, bytes) in [("binary", binary.as_slice()), ("ascii", ascii.as_slice())] {
        let report = import_bytes(&format!("authored-facets-{encoding}.stl"), bytes)?;
        assert!(report.diagnostics().is_empty());
        assert!(report.loss_evidence().is_empty());
        let scene = report.scene();
        assert_eq!(scene.mesh_count(), 1);
        assert_eq!(scene.primitive_count(), 1);
        assert_eq!(scene.attribute_count(), 2);
        let mesh = scene.meshes().next().expect("one STL mesh");
        assert_eq!(mesh.primitive_count(), 1);
        let primitive = mesh.primitives().next().expect("one STL primitive");
        assert_eq!(primitive.topology(), TopologyKind::TriangleList);
        assert_eq!(primitive.vertex_count(), 6);
        assert_eq!(primitive.corner_count(), 6);
        assert_eq!(primitive.face_count(), 2);
        assert_eq!(primitive.indices(), Some(&[0, 1, 2, 3, 4, 5][..]));
        for (attribute, expected) in [
            (
                primitive.positions().expect("STL positions"),
                expected_positions,
            ),
            (
                primitive
                    .normals(AttributeDomain::Vertex)
                    .expect("STL vertex normals"),
                expected_normals,
            ),
        ] {
            assert_eq!(attribute.domain(), AttributeDomain::Vertex);
            assert_eq!(attribute.lanes(), 3);
            assert_eq!(attribute.len(), 6);
            let AttributeData::DenseF32 { lanes: 3, values } = attribute.data() else {
                panic!("expected dense XYZ STL stream, got {:?}", attribute.data());
            };
            assert_eq!(
                values
                    .iter()
                    .map(|value| value.to_bits())
                    .collect::<Vec<_>>(),
                expected.map(f32::to_bits),
                "{encoding} {:?} scalar bits",
                attribute.semantic()
            );
        }
    }
    Ok(())
}

#[test]
fn binary_with_solid_magic_and_arbitrary_bytes_bypasses_ascii_profile() -> TestResult {
    let mut facet = BinaryFacet::unit_triangle();
    facet.attribute = 0xffff;
    let bytes = binary_stl(b"solid\xff\xfe\0binary", &[facet]);
    let report = import_bytes("arbitrary-bytes.stl", &bytes)?;

    let evidence = report
        .detection_evidence()
        .iter()
        .find(|evidence| evidence.format().as_str() == "org.baozi.stl")
        .expect("successful binary import must retain STL detector evidence");
    assert_eq!(
        evidence.outcome(),
        &DetectionOutcome::Match(ReadConfidence::Certain)
    );
    let snapshot = scene_snapshot(&report)?;
    assert_eq!(snapshot.primitive_count(), 1);
    assert_eq!(snapshot.primitive_index_count(), 3);
    assert_eq!(snapshot.f32_scalar_count(), 30);
    assert!(snapshot_line(&snapshot, "attribute[0]").contains("elements=3"));
    Ok(())
}

#[test]
fn zero_facet_binary_stl_is_typed_format_failure() {
    let bytes = binary_stl(b"binary", &[]);
    let failure = import_bytes_with_limits("empty.stl", &bytes, ResourceLimits::default())
        .expect_err("empty binary STL must fail");

    assert_eq!(
        expected_stl_failure(&failure).code(),
        StlFailureCode::EmptyGeometry.as_str()
    );
}

#[test]
fn declared_count_must_match_file_size() {
    let mut bytes = binary_stl(b"binary", &[BinaryFacet::unit_triangle()]);
    bytes.truncate(bytes.len() - 1);
    let failure = import_bytes_with_limits("truncated.stl", &bytes, ResourceLimits::default())
        .expect_err("truncated binary STL must fail");
    assert!(matches!(
        failure.cause(),
        ImportCause::Format(_) | ImportCause::UnsupportedFormat | ImportCause::Detection(_)
    ));
}

#[test]
fn non_finite_binary_float_reaches_canonical_validation() {
    let mut facet = BinaryFacet::unit_triangle();
    facet.vertices[1][0] = f32::NAN;
    let bytes = binary_stl(b"binary", &[facet]);
    let failure = import_bytes_with_limits("nan.stl", &bytes, ResourceLimits::default())
        .expect_err("non-finite geometry must fail validation");

    assert_eq!(
        failure.scene_validation_error(),
        Some(
            &baozi_import::SceneValidationError::NonFiniteAttributeValue {
                attribute: 0,
                scalar: 3,
            }
        )
    );
    let ImportTerminalCause::Failure(terminal) = failure.ledger().terminal_cause() else {
        panic!("expected canonical authoring rejection")
    };
    assert_eq!(terminal.class(), ImportTerminalFailureClass::Requested);
}

#[test]
fn face_limit_is_checked_before_binary_allocation() {
    let bytes = binary_stl(b"binary", &[BinaryFacet::unit_triangle()]);
    let mut limits = ResourceLimits::default();
    limits.max_faces = 0;
    let failure =
        import_bytes_with_limits("limited.stl", &bytes, limits).expect_err("face limit must fail");

    assert!(
        matches!(failure.cause(), ImportCause::Scope(abort) if matches!(abort.cause(),
        baozi_import::ScopeAbortCause::Ledger(baozi_import::LedgerCause::LimitExceeded {
            class: baozi_operation::CounterClass::Attempted, dimension: ImportDimension::Faces, value: 1, limit: 0,
        })))
    );
    for (dimension, attempted) in [
        (ImportDimension::DecodeWork, 134),
        (ImportDimension::Faces, 1),
        (ImportDimension::Vertices, 0),
        (ImportDimension::Meshes, 0),
    ] {
        let row = failure.ledger().dimension(dimension).unwrap();
        assert_eq!(
            (row.attempted(), row.consumed()),
            (attempted, 0),
            "{dimension:?}"
        );
    }
    assert_eq!(
        failure
            .ledger()
            .dimension(ImportDimension::CanonicalOutputBytes)
            .unwrap()
            .peak_live_bytes(),
        0
    );
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
}

#[test]
fn materialise_header_color_maps_to_default_material() -> TestResult {
    let bytes = binary_stl(b"COLOR=\x20\x40\x80\xff", &[BinaryFacet::unit_triangle()]);
    let report = import_bytes("materialise-color.stl", &bytes)?;
    let snapshot = scene_snapshot(&report)?;
    assert!(snapshot_line(&snapshot, "material[0]").contains(concat!(
        "factors(present,total=40)=[",
        "0x00000000,0x00000000,0x00000000,",
        "0x3e008081,0x3e808081,0x3f008081,0x3f800000"
    )));
    Ok(())
}

#[test]
fn binary_facet_colors_expand_to_vertex_color_channel() -> TestResult {
    let mut facet = BinaryFacet::unit_triangle();
    facet.attribute = 0x8000 | (31 << 10);
    let bytes = binary_stl(b"binary", &[facet]);
    let report = import_bytes("colored.stl", &bytes)?;
    let snapshot = scene_snapshot(&report)?;

    assert_eq!(snapshot.attribute_count(), 3);
    assert_eq!(snapshot.f32_scalar_count(), 30);
    assert!(snapshot_line(&snapshot, "attribute[2]").contains(concat!(
        "semantic=color semantic_set=0 domain=vertex scalar=f32 lanes=4 ",
        "normalization=none payload=dense elements=3 "
    )));
    assert!(snapshot_line(&snapshot, "attribute[2]").contains(concat!(
        "values(present,total=12)=[",
        "0x3f800000,0x00000000,0x00000000,0x3f800000"
    )));
    Ok(())
}

#[test]
fn materialise_facet_color_uses_materialise_channel_order() -> TestResult {
    let mut facet = BinaryFacet::unit_triangle();
    facet.attribute = 0x8000 | 31;
    let bytes = binary_stl(b"COLOR=\x20\x40\x80\xff", &[facet]);
    let report = import_bytes("materialise-facet-color.stl", &bytes)?;
    let snapshot = scene_snapshot(&report)?;

    assert!(snapshot_line(&snapshot, "attribute[2]").contains(concat!(
        "values(present,total=12)=[",
        "0x3f800000,0x00000000,0x00000000,0x3f800000"
    )));
    Ok(())
}

#[test]
fn binary_decode_work_envelope_is_exact_and_one_short_denies_before_payload() {
    let bytes = binary_stl(b"binary", &[BinaryFacet::unit_triangle(); 2]);
    assert_eq!(bytes.len(), 184);
    let mut exact = ResourceLimits::default();
    exact.max_decode_work = 184;
    let report = import_bytes_with_limits("binary-exact-envelope.stl", &bytes, exact)
        .expect("exact binary envelope");
    let row = report
        .ledger()
        .dimension(ImportDimension::DecodeWork)
        .unwrap();
    assert_eq!((row.attempted(), row.consumed()), (184, 184));
    let mut below = ResourceLimits::default();
    below.max_decode_work = 183;
    let failure = import_bytes_with_limits("binary-short-envelope.stl", &bytes, below)
        .expect_err("one-short binary envelope");
    assert!(
        matches!(failure.cause(), ImportCause::Scope(abort) if matches!(abort.cause(),
        baozi_import::ScopeAbortCause::Ledger(baozi_import::LedgerCause::LimitExceeded {
            class: baozi_operation::CounterClass::Attempted, dimension: ImportDimension::DecodeWork, value: 184, limit: 183,
        })))
    );
    for (dimension, attempted) in [
        (ImportDimension::DecodeWork, 184),
        (ImportDimension::Faces, 0),
        (ImportDimension::Vertices, 0),
        (ImportDimension::Records, 0),
        (ImportDimension::Meshes, 0),
    ] {
        let row = failure.ledger().dimension(dimension).unwrap();
        assert_eq!(
            (row.attempted(), row.consumed()),
            (attempted, 0),
            "{dimension:?}"
        );
    }
    assert_eq!(
        failure
            .ledger()
            .dimension(ImportDimension::CanonicalOutputBytes)
            .unwrap()
            .peak_live_bytes(),
        0
    );
    assert_eq!(failure.ledger().current_live_bytes(), 0);
    assert_eq!(failure.ledger().active_reservations(), 0);
}
