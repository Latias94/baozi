#![cfg(feature = "format-obj")]

use baozi::{ImportDimension, ImportLedgerSnapshot, ImportReport, Importer, PrimitiveTopology};
use baozi_test_support::{ObjScale, ObjScaleFixture};

fn assert_counter(report: &ImportReport, dimension: ImportDimension, expected: u64) {
    let evidence = report
        .ledger()
        .dimension(dimension)
        .expect("scale contract dimension should exist");
    assert_eq!(evidence.attempted(), expected, "{dimension:?} attempted");
    assert_eq!(evidence.consumed(), expected, "{dimension:?} consumed");
}

fn assert_clean_terminal_ledger(ledger: &ImportLedgerSnapshot) {
    assert_eq!(ledger.current_live_bytes(), 0);
    assert_eq!(ledger.active_reservations(), 0);
    assert_eq!(ledger.current_evidence_bytes(), 0);

    for (dimension, evidence) in ledger.dimensions() {
        assert!(
            evidence.consumed() <= evidence.attempted(),
            "{dimension:?} consumed work must not exceed attempted work"
        );
        assert_eq!(
            evidence.current_live_bytes(),
            0,
            "{dimension:?} retained live bytes"
        );
        assert_eq!(
            evidence.pending_publication(),
            0,
            "{dimension:?} pending publication"
        );
    }
    for (phase, evidence) in ledger.phases() {
        assert_eq!(
            evidence.current_live_bytes(),
            0,
            "{phase:?} retained live bytes"
        );
    }
}

#[test]
fn obj_scale_work_is_affine_and_declared_live_memory_stays_inside_reviewed_envelope() {
    let importer = Importer::new().expect("default format registry should freeze");

    for scale in ObjScale::CORRECTNESS {
        let fixture =
            ObjScaleFixture::try_new(scale).expect("bounded scale fixture should allocate");
        let report = importer
            .read_bytes("scale.obj", fixture.bytes())
            .expect("deterministic OBJ fixture should import");

        assert_eq!(report.format().id().as_str(), "org.baozi.obj");
        assert!(report.diagnostics().is_empty());
        assert_eq!(report.scene().meshes().len(), 1);
        let mesh = &report.scene().meshes()[0];
        assert_eq!(mesh.topology, PrimitiveTopology::Triangles);
        assert_eq!(mesh.positions.len(), 3);
        assert_eq!(mesh.indices.len(), fixture.face_count() * 3);

        assert_counter(
            &report,
            ImportDimension::PrimaryBytes,
            fixture.bytes().len() as u64,
        );
        assert_counter(
            &report,
            ImportDimension::Records,
            fixture.expected_record_count(),
        );
        assert_counter(
            &report,
            ImportDimension::Tokens,
            fixture.expected_token_count(),
        );
        assert_counter(&report, ImportDimension::Faces, fixture.face_count() as u64);
        assert_clean_terminal_ledger(report.ledger());
        assert!(report.ledger().peak_live_bytes() > 0);
        assert!(
            report.ledger().peak_live_bytes() <= fixture.reviewed_peak_live_envelope(),
            "{} observed {} declared peak bytes above reviewed affine envelope {}",
            scale.label(),
            report.ledger().peak_live_bytes(),
            fixture.reviewed_peak_live_envelope()
        );
    }
}
