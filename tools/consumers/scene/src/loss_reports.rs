//! Loss policy and retained report consumption through the public facade.

use baozi::{
    AttributeData, AttributeDomain, AttributeSemanticRef, DiagnosticByteRange, ImportCause,
    ImportConfigOverlay, ImportDiagnostic, ImportEvidence, ImportLossDisposition,
    ImportLossEvidence, ImportLossKind, ImportLossSubject, ImportOperationControl, ImportRequest,
    ImportRequestLayer, Importer, LossPolicy, OptionCompileLimits, ScopeAbortCause,
    SourceScopeEntityKind, ValidatedScene,
};

const SOURCE: &str = "partial-uv.obj";
const PARTIAL_UV: &[u8] =
    b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvn 0 0 1\nf 1/1/1 2//1 3/3/1\n";

pub(super) fn verify(importer: &Importer) -> Result<(), Box<dyn std::error::Error>> {
    let failure = match importer.read_bytes(SOURCE, PARTIAL_UV) {
        Err(baozi::Error::Import(failure)) => failure,
        other => panic!("strict partial-UV import must retain its failure: {other:?}"),
    };
    assert!(matches!(
        failure.cause(),
        ImportCause::Scope(abort)
            if matches!(
                abort.cause(),
                ScopeAbortCause::SourceSemanticLossRejectedByLossPolicy { code }
                    if code.as_str() == "obj.partial_texcoord_channel"
            )
    ));
    assert_loss(
        failure.loss_evidence(),
        failure.diagnostics(),
        ImportLossDisposition::RejectedByLossPolicy,
    );
    assert_eq!(failure.ledger().committed_canonical_members(), 0);
    drop(failure);

    let request = ImportRequest::new().with_caller_layer(ImportRequestLayer::new(
        ImportConfigOverlay::default().with_loss_policy(
            LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss),
        ),
    ));
    let prepared = importer.prepare_request(&request, OptionCompileLimits::default())?;
    let report = importer.read_bytes_with_request(
        SOURCE,
        PARTIAL_UV,
        &prepared,
        ImportOperationControl::default(),
    )?;
    let (scene, evidence): (ValidatedScene, ImportEvidence) = report.into_parts();
    assert!(evidence.matches_scene(&scene));
    assert_eq!(evidence.source().as_str(), SOURCE);
    let profile: &baozi::TextProfile = evidence
        .format()
        .text_profile()
        .expect("OBJ declares its text profile");
    assert_eq!(profile.bom_policy(), baozi::TextBomPolicy::Accepted);
    assert_eq!(
        profile.lexical_mode(),
        baozi::TextLexicalMode::AsciiSyntaxUtf8Values
    );
    assert_eq!(
        profile.source_encodings().collect::<Vec<_>>(),
        [baozi::TextSourceEncoding::Utf8]
    );
    assert!(profile.allows_decoding_mode(baozi::TextDecodingMode::RejectInvalid));
    let caps: &baozi::TextHardCaps = profile.hard_caps();
    assert!(caps.max_raw_bytes() >= PARTIAL_UV.len() as u64);
    let mesh = scene.meshes().next().expect("one retained mesh");
    let primitive = mesh.primitives().next().expect("one retained triangle");
    assert_eq!(scene.primitive_count(), 1);
    assert_eq!(primitive.indices(), Some(&[0, 1, 2][..]));
    let AttributeData::DenseF32 { lanes: 3, values } =
        primitive.positions().expect("retained positions").data()
    else {
        panic!("dense XYZ positions expected");
    };
    assert_eq!(values, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
    let normal = primitive
        .attribute(AttributeSemanticRef::Normal, AttributeDomain::Corner)
        .expect("complete authored normals must survive UV loss");
    let AttributeData::DenseF32 { lanes: 3, values } = normal.data() else {
        panic!("dense corner normals expected");
    };
    assert_eq!(values, &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
    assert!(
        primitive
            .attributes()
            .all(|attribute| attribute.semantic() != AttributeSemanticRef::Texcoord(0))
    );
    drop(scene);
    let work: baozi::ImportDimensionSnapshot = evidence
        .ledger()
        .dimension(baozi::ImportDimension::WorkUnits)
        .expect("successful import retains its resource report");
    assert!(work.consumed() > 0);
    assert_eq!(work.attempted(), work.consumed());
    assert_eq!(work.current_live_bytes(), 0);
    assert_loss(
        evidence.loss_evidence(),
        evidence.diagnostics(),
        ImportLossDisposition::Applied,
    );
    Ok(())
}

fn assert_loss(
    losses: &[ImportLossEvidence],
    diagnostics: &[ImportDiagnostic],
    disposition: ImportLossDisposition,
) {
    assert_eq!(losses.len(), 1);
    let loss = &losses[0];
    assert_eq!(loss.kind(), ImportLossKind::SourceSemanticLoss);
    assert_eq!(
        loss.subject(),
        ImportLossSubject::SourceEntity(SourceScopeEntityKind::Mesh)
    );
    assert_eq!(loss.code().as_str(), "obj.partial_texcoord_channel");
    assert_eq!(loss.source().as_str(), SOURCE);
    let range: Option<DiagnosticByteRange> = loss.raw_range();
    assert_eq!(range, None);
    assert_eq!(loss.disposition(), disposition);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code(), loss.code());
    assert_eq!(diagnostics[0].source().as_str(), SOURCE);
    assert_eq!(
        diagnostics[0].object_path(),
        Some("/segments/0/attributes/texcoord")
    );
}
