mod common;

use baozi_core::AttributeData;
use baozi_import::authoring::AttributeDomain;
use baozi_import::{ImportReport, SceneSnapshot};
use common::{TestResult, import_bytes, scene_snapshot, snapshot_line_containing};

const FORMATS: [&str; 3] = ["ascii", "binary_little_endian", "binary_big_endian"];
const PROPERTY_NAMES: [&str; 8] = ["x", "y", "z", "nx", "ny", "nz", "s", "t"];
const STREAMS: [(&str, usize, usize); 3] =
    [("position", 0, 3), ("normal", 3, 3), ("texcoord", 6, 2)];
const F64_TRIPLES: [[f64; 3]; 3] = [
    [16_777_216.0, 16_777_217.0, 1e-100],
    [f64::from_bits(1), -0.0, 0.0],
    [-f64::from_bits(1), -1e-100, -16_777_217.0],
];
const F64_PAIRS: [[f64; 2]; 4] = [
    [16_777_216.0, 16_777_217.0],
    [f64::from_bits(1), 1e-100],
    [-0.0, 0.0],
    [-f64::from_bits(1), -1e-100],
];

fn fixture(format: &str, properties: &[(&str, &str)], rows: &[&[f64]]) -> Vec<u8> {
    let mut header = format!("ply\nformat {format} 1.0\nelement vertex {}\n", rows.len());
    for (scalar, name) in properties {
        header.push_str(&format!("property {scalar} {name}\n"));
    }
    header.push_str("end_header\n");
    let mut bytes = header.into_bytes();
    for row in rows {
        assert_eq!(row.len(), properties.len());
        for ((scalar, _), value) in properties.iter().zip(*row) {
            if format == "ascii" {
                bytes.extend_from_slice(format!("{value} ").as_bytes());
                continue;
            }
            macro_rules! append_scalar {
                ($value:expr) => {
                    bytes.extend_from_slice(&if format == "binary_little_endian" {
                        ($value).to_le_bytes()
                    } else {
                        ($value).to_be_bytes()
                    })
                };
            }
            match *scalar {
                "char" | "int8" => append_scalar!(*value as i8),
                "uchar" | "uint8" => append_scalar!(*value as u8),
                "short" | "int16" => append_scalar!(*value as i16),
                "ushort" | "uint16" => append_scalar!(*value as u16),
                "int" | "int32" => append_scalar!(*value as i32),
                "uint" | "uint32" => append_scalar!(*value as u32),
                "float" | "float32" => append_scalar!(*value as f32),
                "double" | "float64" => append_scalar!(*value),
                scalar => panic!("unsupported fixture scalar: {scalar}"),
            }
        }
        if format == "ascii" {
            bytes.push(b'\n');
        }
    }
    bytes
}

fn import_case(bytes: &[u8]) -> TestResult<(ImportReport, SceneSnapshot)> {
    let report = import_bytes("source-precision.ply", bytes)?;
    assert!(report.diagnostics().is_empty());
    assert!(report.loss_evidence().is_empty());
    assert_eq!(report.ledger().current_live_bytes(), 0);
    assert_eq!(report.ledger().active_reservations(), 0);
    let snapshot = scene_snapshot(&report)?;
    Ok((report, snapshot))
}

fn assert_stream(
    report: &ImportReport,
    snapshot: &SceneSnapshot,
    semantic: &str,
    lanes: usize,
    scalar: &str,
    expected: &[f64],
) {
    let primitive = report
        .scene()
        .meshes()
        .next()
        .expect("PLY has one mesh")
        .primitives()
        .next()
        .expect("PLY has one primitive");
    let attribute = match semantic {
        "position" => primitive.positions(),
        "normal" => primitive.normals(AttributeDomain::Vertex),
        "texcoord" => primitive.texcoords(0, AttributeDomain::Vertex),
        semantic => panic!("unsupported fixture semantic: {semantic}"),
    }
    .expect("declared PLY stream must be retained");
    assert_eq!(attribute.domain(), AttributeDomain::Vertex);
    assert_eq!(usize::from(attribute.lanes()), lanes);
    assert_eq!(attribute.len(), expected.len() / lanes);
    match (scalar, attribute.data()) {
        (
            "f64",
            AttributeData::DenseF64 {
                lanes: actual_lanes,
                values,
            },
        ) => {
            assert_eq!(usize::from(actual_lanes), lanes);
            assert_eq!(
                values
                    .iter()
                    .map(|value| value.to_bits())
                    .collect::<Vec<_>>(),
                expected
                    .iter()
                    .map(|value| value.to_bits())
                    .collect::<Vec<_>>(),
                "{semantic} F64 bits"
            );
        }
        (
            "f32",
            AttributeData::DenseF32 {
                lanes: actual_lanes,
                values,
            },
        ) => {
            assert_eq!(usize::from(actual_lanes), lanes);
            assert_eq!(
                values
                    .iter()
                    .map(|value| value.to_bits())
                    .collect::<Vec<_>>(),
                expected
                    .iter()
                    .map(|value| (*value as f32).to_bits())
                    .collect::<Vec<_>>(),
                "{semantic} F32 bits"
            );
        }
        (scalar, payload) => panic!("expected dense {scalar} {semantic}, got {payload:?}"),
    }

    let attribute = snapshot_line_containing(snapshot, &format!("semantic={semantic} "));
    assert!(
        attribute.contains(&format!(
            "semantic={semantic} semantic_set=0 domain=vertex scalar={scalar} lanes={lanes} \
             normalization=none payload=dense elements={}",
            expected.len() / lanes
        )),
        "{attribute}"
    );
    let bits = expected
        .iter()
        .map(|value| match scalar {
            "f64" => format!("0x{:016x}", value.to_bits()),
            "f32" => format!("0x{:08x}", (*value as f32).to_bits()),
            scalar => panic!("unsupported expected scalar: {scalar}"),
        })
        .collect::<Vec<_>>()
        .join(",");
    assert!(
        attribute.contains(&format!(
            "values(present,total={})=[{bits}]",
            expected.len()
        )),
        "{attribute}"
    );
}

fn assert_f64_stream(semantic: &str, names: &[&str], values: &[&[f64]]) -> TestResult<()> {
    let mut properties = Vec::new();
    if semantic != "position" {
        properties.extend([("float", "x"), ("float", "y"), ("float", "z")]);
    }
    for (index, name) in names.iter().enumerate() {
        let scalar = if index % 2 == 0 { "double" } else { "float64" };
        properties.push((scalar, *name));
    }
    let rows = values
        .iter()
        .map(|values| {
            let mut row = Vec::new();
            if semantic != "position" {
                row.extend([0.0, 0.0, 0.0]);
            }
            row.extend_from_slice(values);
            row
        })
        .collect::<Vec<_>>();
    let row_slices = rows.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let expected = values
        .iter()
        .flat_map(|row| row.iter().copied())
        .collect::<Vec<_>>();
    for format in FORMATS {
        let (report, snapshot) = import_case(&fixture(format, &properties, &row_slices))?;
        assert_stream(&report, &snapshot, semantic, names.len(), "f64", &expected);
        if semantic != "position" {
            assert_stream(
                &report,
                &snapshot,
                "position",
                3,
                "f32",
                &vec![0.0; rows.len() * 3],
            );
        }
    }
    Ok(())
}

#[test]
fn source_precision_f64_positions_preserve_bits_in_all_encodings() -> TestResult<()> {
    let rows = F64_TRIPLES.iter().map(|row| &row[..]).collect::<Vec<_>>();
    assert_f64_stream("position", &["x", "y", "z"], &rows)
}

#[test]
fn source_precision_f64_normals_preserve_bits_in_all_encodings() -> TestResult<()> {
    let rows = F64_TRIPLES.iter().map(|row| &row[..]).collect::<Vec<_>>();
    assert_f64_stream("normal", &["nx", "ny", "nz"], &rows)
}

#[test]
fn source_precision_f64_texcoords_preserve_bits_in_all_encodings() -> TestResult<()> {
    let rows = F64_PAIRS.iter().map(|row| &row[..]).collect::<Vec<_>>();
    for names in [["s", "t"], ["u", "v"], ["texture_u", "texture_v"]] {
        assert_f64_stream("texcoord", &names, &rows)?;
    }
    Ok(())
}

#[test]
fn source_precision_f64_outside_f32_range_is_retained_in_all_encodings() -> TestResult<()> {
    assert_f64_stream("position", &["x", "y", "z"], &[&[1e100, -1e100, -0.0]])?;
    assert_f64_stream("normal", &["nx", "ny", "nz"], &[&[1e100, -1e100, -0.0]])?;
    assert_f64_stream("texcoord", &["s", "t"], &[&[1e100, -1e100]])
}

#[test]
fn source_precision_i32_u32_boundaries_are_exact_in_all_encodings() -> TestResult<()> {
    for (scalar, low, high) in [
        ("int", f64::from(i32::MIN), f64::from(i32::MAX)),
        ("int32", f64::from(i32::MIN), f64::from(i32::MAX)),
        ("uint", 0.0, f64::from(u32::MAX)),
        ("uint32", 0.0, f64::from(u32::MAX)),
    ] {
        let properties = PROPERTY_NAMES.map(|name| (scalar, name));
        let rows = [
            [low, high, 16_777_217.0, low, high, 16_777_217.0, low, high],
            [16_777_216.0; 8],
        ];
        for format in FORMATS {
            let (report, snapshot) =
                import_case(&fixture(format, &properties, &[&rows[0], &rows[1]]))?;
            for (semantic, start, lanes) in STREAMS {
                let expected = rows
                    .iter()
                    .flat_map(|row| row[start..start + lanes].iter().copied())
                    .collect::<Vec<_>>();
                assert_stream(&report, &snapshot, semantic, lanes, "f64", &expected);
            }
        }
    }
    Ok(())
}

#[test]
fn source_precision_mixed_components_choose_declared_width_per_stream() -> TestResult<()> {
    let row = [1.0, -0.0, 1.0, 0.0, 1.0, 0.0, -0.0, 1.0];
    for (wide_semantic, start, lanes) in STREAMS {
        for scalar in ["double", "int", "uint"] {
            let mut properties = PROPERTY_NAMES.map(|name| ("float", name));
            // A late wide component selects the whole stream even when every value fits F32.
            properties[start + lanes - 1].0 = scalar;
            for format in FORMATS {
                let (report, snapshot) = import_case(&fixture(format, &properties, &[&row]))?;
                for (semantic, start, lanes) in STREAMS {
                    let expected_scalar = if semantic == wide_semantic {
                        "f64"
                    } else {
                        "f32"
                    };
                    assert_stream(
                        &report,
                        &snapshot,
                        semantic,
                        lanes,
                        expected_scalar,
                        &row[start..start + lanes],
                    );
                }
            }
        }
    }
    Ok(())
}

#[test]
fn source_precision_mixed_declared_components_preserve_exact_values() -> TestResult<()> {
    let properties = [
        ("float", "x"),
        ("double", "y"),
        ("short", "z"),
        ("int", "nx"),
        ("float", "ny"),
        ("uchar", "nz"),
        ("float", "s"),
        ("uint", "t"),
    ];
    let row = [
        -0.0,
        1e-100,
        f64::from(i16::MIN),
        f64::from(i32::MAX),
        -0.0,
        255.0,
        f64::from(f32::from_bits(1)),
        f64::from(u32::MAX),
    ];
    for format in FORMATS {
        let (report, snapshot) = import_case(&fixture(format, &properties, &[&row]))?;
        for (semantic, start, lanes) in STREAMS {
            assert_stream(
                &report,
                &snapshot,
                semantic,
                lanes,
                "f64",
                &row[start..start + lanes],
            );
        }
    }
    Ok(())
}

#[test]
fn source_precision_f32_retains_declared_rounding_and_bits() -> TestResult<()> {
    let properties = PROPERTY_NAMES.map(|name| ("float", name));
    let row = [
        16_777_217.0,
        f64::from(f32::from_bits(1)),
        -0.0,
        -0.0,
        f64::from(f32::from_bits(1)),
        1.0,
        -0.0,
        16_777_217.0,
    ];
    for scalar in ["float", "float32"] {
        let properties = properties.map(|(_, name)| (scalar, name));
        for format in FORMATS {
            let (report, snapshot) = import_case(&fixture(format, &properties, &[&row]))?;
            for (semantic, start, lanes) in STREAMS {
                assert_stream(
                    &report,
                    &snapshot,
                    semantic,
                    lanes,
                    "f32",
                    &row[start..start + lanes],
                );
            }
        }
    }
    Ok(())
}

#[test]
fn source_precision_small_integer_declarations_remain_f32() -> TestResult<()> {
    for (scalar, low, high) in [
        ("char", -128.0, 127.0),
        ("int8", -128.0, 127.0),
        ("uchar", 0.0, 255.0),
        ("uint8", 0.0, 255.0),
        ("short", -32_768.0, 32_767.0),
        ("int16", -32_768.0, 32_767.0),
        ("ushort", 0.0, 65_535.0),
        ("uint16", 0.0, 65_535.0),
    ] {
        let properties = PROPERTY_NAMES.map(|name| (scalar, name));
        let row = [low, high, 1.0, low, high, 1.0, low, high];
        for format in FORMATS {
            let (report, snapshot) = import_case(&fixture(format, &properties, &[&row]))?;
            for (semantic, start, lanes) in STREAMS {
                assert_stream(
                    &report,
                    &snapshot,
                    semantic,
                    lanes,
                    "f32",
                    &row[start..start + lanes],
                );
            }
        }
    }
    Ok(())
}
