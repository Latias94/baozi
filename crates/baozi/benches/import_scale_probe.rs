use baozi::{
    Error as BaoziError, ImportDimension, Importer, SceneSnapshotError, SceneSnapshotLimits,
};
use baozi_test_support::{ObjScale, ObjScaleFixture, ObjScaleFixtureError};
use std::error::Error;
use std::fmt;

const PROBE_SCHEMA: &str = "baozi-scale-probe/v1";

#[derive(Debug)]
enum ProbeError {
    Usage,
    UnsupportedScale(String),
    Fixture(ObjScaleFixtureError),
    Import(Box<BaoziError>),
    Snapshot(SceneSnapshotError),
    MissingDimension(ImportDimension),
    TerminalInvariant(&'static str),
    PeakEnvelope { observed: u64, limit: u64 },
    SceneShape,
}

impl fmt::Display for ProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage => formatter.write_str("usage: import_scale_probe <idle|1|10|100|1000>"),
            Self::UnsupportedScale(value) => write!(formatter, "unsupported scale: {value}"),
            Self::Fixture(source) => fmt::Display::fmt(source, formatter),
            Self::Import(source) => fmt::Display::fmt(source, formatter),
            Self::Snapshot(source) => fmt::Display::fmt(source, formatter),
            Self::MissingDimension(dimension) => {
                write!(formatter, "missing ledger dimension: {dimension:?}")
            }
            Self::TerminalInvariant(invariant) => {
                write!(formatter, "terminal ledger invariant failed: {invariant}")
            }
            Self::PeakEnvelope { observed, limit } => write!(
                formatter,
                "declared peak live bytes {observed} exceed reviewed envelope {limit}"
            ),
            Self::SceneShape => {
                formatter.write_str("imported OBJ scene shape is not deterministic")
            }
        }
    }
}

impl Error for ProbeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Fixture(source) => Some(source),
            Self::Import(source) => Some(source.as_ref()),
            Self::Snapshot(source) => Some(source),
            _ => None,
        }
    }
}

impl From<ObjScaleFixtureError> for ProbeError {
    fn from(source: ObjScaleFixtureError) -> Self {
        Self::Fixture(source)
    }
}

impl From<BaoziError> for ProbeError {
    fn from(source: BaoziError) -> Self {
        Self::Import(Box::new(source))
    }
}

impl From<SceneSnapshotError> for ProbeError {
    fn from(source: SceneSnapshotError) -> Self {
        Self::Snapshot(source)
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("import scale probe failed: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), ProbeError> {
    let mut arguments = std::env::args().skip(1);
    let requested = arguments.next().ok_or(ProbeError::Usage)?;
    if arguments.next().is_some() {
        return Err(ProbeError::Usage);
    }

    let importer = Importer::new()?;
    if requested == "idle" {
        let _ = std::hint::black_box(&importer);
        println!("schema={PROBE_SCHEMA}");
        println!("case_id=idle_default_registry");
        println!("scale=0");
        println!("scale_label=idle");
        return Ok(());
    }

    let multiplier = requested
        .parse::<usize>()
        .map_err(|_| ProbeError::UnsupportedScale(requested.clone()))?;
    let scale = ObjScale::from_multiplier(multiplier)
        .ok_or_else(|| ProbeError::UnsupportedScale(requested.clone()))?;
    let fixture = ObjScaleFixture::try_new(scale)?;
    let report = importer.read_bytes("scale.obj", fixture.bytes())?;
    let ledger = report.ledger();
    let snapshot_limits = SceneSnapshotLimits::try_new(64 * 1024, 1 << 12, 4_000_000)?;
    let snapshot = report.scene().capture_snapshot(snapshot_limits)?;

    if ledger.current_live_bytes() != 0 {
        return Err(ProbeError::TerminalInvariant("current_live_bytes"));
    }
    if ledger.active_reservations() != 0 {
        return Err(ProbeError::TerminalInvariant("active_reservations"));
    }
    if ledger.current_evidence_bytes() != 0 {
        return Err(ProbeError::TerminalInvariant("current_evidence_bytes"));
    }
    let dimension = |key| {
        ledger
            .dimension(key)
            .ok_or(ProbeError::MissingDimension(key))
    };
    let primary = dimension(ImportDimension::PrimaryBytes)?;
    let records = dimension(ImportDimension::Records)?;
    let tokens = dimension(ImportDimension::Tokens)?;
    let faces = dimension(ImportDimension::Faces)?;
    let canonical_output = dimension(ImportDimension::CanonicalOutputBytes)?;
    for evidence in [primary, records, tokens, faces, canonical_output] {
        if evidence.consumed() > evidence.attempted() {
            return Err(ProbeError::TerminalInvariant("consumed_above_attempted"));
        }
        if evidence.current_live_bytes() != 0 {
            return Err(ProbeError::TerminalInvariant(
                "dimension_current_live_bytes",
            ));
        }
    }

    let reviewed_peak = fixture.reviewed_peak_live_envelope();
    if ledger.peak_live_bytes() > reviewed_peak {
        return Err(ProbeError::PeakEnvelope {
            observed: ledger.peak_live_bytes(),
            limit: reviewed_peak,
        });
    }

    let expected_indices = u64::try_from(fixture.face_count())
        .ok()
        .and_then(|faces| faces.checked_mul(3))
        .ok_or(ProbeError::SceneShape)?;
    if snapshot.logical_mesh_count() != 1
        || snapshot.primitive_index_count() != expected_indices
        || snapshot.attribute_count() < 1
    {
        return Err(ProbeError::SceneShape);
    }

    println!("schema={PROBE_SCHEMA}");
    println!("case_id={}", ObjScaleFixture::CASE_ID);
    println!("scale={}", scale.multiplier());
    println!("scale_label={}", scale.label());
    println!("logical_units={}", fixture.face_count());
    println!("input_bytes={}", fixture.bytes().len());
    println!("scene_meshes={}", snapshot.logical_mesh_count());
    println!("scene_indices={}", snapshot.primitive_index_count());
    println!("ledger_current_live_bytes={}", ledger.current_live_bytes());
    println!("ledger_peak_live_bytes={}", ledger.peak_live_bytes());
    println!(
        "ledger_active_reservations={}",
        ledger.active_reservations()
    );
    println!(
        "ledger_current_evidence_bytes={}",
        ledger.current_evidence_bytes()
    );
    println!(
        "ledger_peak_evidence_bytes={}",
        ledger.peak_evidence_bytes()
    );
    println!("reviewed_peak_live_envelope_bytes={reviewed_peak}");
    println!("primary_bytes_attempted={}", primary.attempted());
    println!("primary_bytes_consumed={}", primary.consumed());
    println!("records_attempted={}", records.attempted());
    println!("records_consumed={}", records.consumed());
    println!("tokens_attempted={}", tokens.attempted());
    println!("tokens_consumed={}", tokens.consumed());
    println!("faces_attempted={}", faces.attempted());
    println!("faces_consumed={}", faces.consumed());
    println!(
        "canonical_output_committed={}",
        canonical_output.committed()
    );
    Ok(())
}
