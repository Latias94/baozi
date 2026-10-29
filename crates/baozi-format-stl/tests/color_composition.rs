mod common;

use baozi_core::{AlphaMode, AttributeData, AttributeDomain, AttributeSemanticRef, Color};
use baozi_import::ImportReport;
use common::{BinaryFacet, binary_stl, import_bytes};
use std::error::Error;

type TestResult = Result<(), Box<dyn Error + Send + Sync>>;

fn facet(attribute: u16) -> BinaryFacet {
    BinaryFacet {
        attribute,
        ..BinaryFacet::unit_triangle()
    }
}

fn color_components(color: Color) -> [f32; 4] {
    [color.r, color.g, color.b, color.a]
}

fn assert_rgba(actual: [f32; 4], expected: [f32; 4]) {
    for (lane, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
        assert!(
            (actual - expected).abs() <= 1e-6,
            "RGBA lane {lane}: expected {expected}, got {actual}"
        );
    }
}

fn assert_composition(
    report: &ImportReport,
    expected_material: [f32; 4],
    expected_facets: &[[f32; 4]],
    has_vertex_colors: bool,
    expected_alpha: AlphaMode,
) {
    // The common import helper uses strict conformance and preserve-or-fail loss policy.
    assert!(report.diagnostics().is_empty());
    assert!(report.loss_evidence().is_empty());
    let scene = report.scene();
    assert_eq!(scene.material_count(), 1);
    assert_eq!(scene.mesh_count(), 1);
    assert_eq!(scene.primitive_count(), 1);
    let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
    assert_eq!(primitive.vertex_count() as usize, 3 * expected_facets.len());
    let material = primitive.material().expect("STL material binding");
    assert_eq!(material.alpha_mode(), expected_alpha);
    let factors = material.factors();
    assert!(factors.legacy().is_none());
    let base_color = color_components(
        factors
            .pbr_metallic_roughness()
            .expect("STL base-color factors")
            .base_color(),
    );
    assert_rgba(base_color, expected_material);

    let colors = primitive.attribute(AttributeSemanticRef::Color(0));
    assert_eq!(colors.is_some(), has_vertex_colors);
    let colors = colors.map(|attribute| {
        assert_eq!(attribute.domain(), AttributeDomain::Vertex);
        assert_eq!(attribute.len(), 3 * expected_facets.len());
        match attribute.data() {
            AttributeData::DenseF32 { lanes: 4, values } => {
                assert_eq!(values.len(), 12 * expected_facets.len());
                values
            }
            payload => panic!("expected dense RGBA vertex colors, got {payload:?}"),
        }
    });

    // Evaluate the actual typed material factor and every actual vertex color together.
    for (face_index, expected) in expected_facets.iter().copied().enumerate() {
        for corner in 0..3 {
            let vertex = face_index * 3 + corner;
            let vertex_color: [f32; 4] = colors.map_or([1.0; 4], |values| {
                values[vertex * 4..vertex * 4 + 4].try_into().unwrap()
            });
            if has_vertex_colors {
                assert_rgba(vertex_color, expected);
            }
            let combined = std::array::from_fn(|lane| base_color[lane] * vertex_color[lane]);
            assert_rgba(combined, expected);
        }
    }
}

#[test]
fn uncolored_stl_uses_neutral_material_without_vertex_colors() -> TestResult {
    let bytes = binary_stl(b"binary", &[facet(0), facet(0x7fff)]);
    let report = import_bytes("uncolored.stl", &bytes)?;

    assert_composition(&report, [1.0; 4], &[[1.0; 4]; 2], false, AlphaMode::Opaque);
    Ok(())
}

#[test]
fn object_color_without_facet_colors_is_applied_once() -> TestResult {
    for alpha in [0, 97, 255] {
        let bytes = binary_stl(
            &[b'C', b'O', b'L', b'O', b'R', b'=', 64, 128, 192, alpha],
            &[facet(0), facet(0x7fff)],
        );
        let report = import_bytes("object-color.stl", &bytes)?;
        let source = [
            64.0 / 255.0,
            128.0 / 255.0,
            192.0 / 255.0,
            f32::from(alpha) / 255.0,
        ];
        let alpha_mode = if alpha == 255 {
            AlphaMode::Opaque
        } else {
            AlphaMode::Blend
        };

        assert_composition(&report, source, &[source; 2], false, alpha_mode);
    }
    Ok(())
}

#[test]
fn mixed_magics_colors_override_object_fallback_without_multiplying_rgba() -> TestResult {
    for alpha in [0, 128, 255] {
        let bytes = binary_stl(
            &[b'C', b'O', b'L', b'O', b'R', b'=', 0, 96, 192, alpha],
            &[
                facet(0),
                facet(0x8000 | 19 | (7 << 5) | (3 << 10)),
                facet(0x7fff),
                facet(0x8000),
            ],
        );
        let report = import_bytes("mixed-magics.stl", &bytes)?;
        let fallback = [0.0, 96.0 / 255.0, 192.0 / 255.0, f32::from(alpha) / 255.0];
        let source = [
            fallback,
            [19.0 / 31.0, 7.0 / 31.0, 3.0 / 31.0, 1.0],
            fallback,
            [0.0, 0.0, 0.0, 1.0],
        ];
        let alpha_mode = if alpha == 255 {
            AlphaMode::Opaque
        } else {
            AlphaMode::Blend
        };

        assert_composition(&report, [1.0; 4], &source, true, alpha_mode);
    }
    Ok(())
}

#[test]
fn mixed_viscam_colors_use_white_fallback() -> TestResult {
    let bytes = binary_stl(
        b"binary",
        &[
            facet(0),
            facet(0x8000 | (19 << 10) | (7 << 5) | 3),
            facet(0x7fff),
            facet(0x8000),
        ],
    );
    let report = import_bytes("mixed-viscam.stl", &bytes)?;
    let source = [
        [1.0; 4],
        [19.0 / 31.0, 7.0 / 31.0, 3.0 / 31.0, 1.0],
        [1.0; 4],
        [0.0, 0.0, 0.0, 1.0],
    ];

    assert_composition(&report, [1.0; 4], &source, true, AlphaMode::Opaque);
    Ok(())
}

#[test]
fn all_magics_facet_colors_ignore_object_tint_and_transparency() -> TestResult {
    for alpha in [0, 113, 255] {
        let bytes = binary_stl(
            &[b'C', b'O', b'L', b'O', b'R', b'=', 32, 64, 128, alpha],
            &[
                facet(0x8000 | 19 | (7 << 5) | (3 << 10)),
                facet(0x8000 | 2 | (15 << 5) | (29 << 10)),
            ],
        );
        let report = import_bytes("all-magics.stl", &bytes)?;
        let source = [
            [19.0 / 31.0, 7.0 / 31.0, 3.0 / 31.0, 1.0],
            [2.0 / 31.0, 15.0 / 31.0, 29.0 / 31.0, 1.0],
        ];

        assert_composition(&report, [1.0; 4], &source, true, AlphaMode::Opaque);
    }
    Ok(())
}

#[test]
fn all_viscam_facet_colors_preserve_nonwhite_components() -> TestResult {
    let bytes = binary_stl(
        b"binary",
        &[
            facet(0x8000 | (19 << 10) | (7 << 5) | 3),
            facet(0x8000 | (2 << 10) | (15 << 5) | 29),
        ],
    );
    let report = import_bytes("all-viscam.stl", &bytes)?;
    let source = [
        [19.0 / 31.0, 7.0 / 31.0, 3.0 / 31.0, 1.0],
        [2.0 / 31.0, 15.0 / 31.0, 29.0 / 31.0, 1.0],
    ];

    assert_composition(&report, [1.0; 4], &source, true, AlphaMode::Opaque);
    Ok(())
}
