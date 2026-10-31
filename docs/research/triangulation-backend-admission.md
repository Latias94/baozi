# Triangulation Backend Admission

**Decision date:** 2026-07-10

**Evidence captured:** 2026-07-10T08:20:16Z

**Decision:** Admit GeoRust `earcut` 0.4.10 for U7 behind the backend-neutral contract in ADR 0013.

## Executive Result

GeoRust's current package is [`earcut`](https://crates.io/crates/earcut/0.4.10), not `earcutr`.
Version 0.4.10 satisfies Baozi's `f64`, license, Rust 1.95, WASM, maintenance, and small-dependency
gates. It correctly triangulated the tested concave simple polygons and retained source indices.

It is not safe as a validator. It returned successful-looking indices for self-intersecting, NaN,
and infinite input, normalized both source windings to counter-clockwise triangles, and panicked for
malformed hole offsets. Admission therefore applies only to an already finite, planar, simple,
bounded single ring. Baozi must validate and project input, recover source winding, validate output,
and publish all corner/face remaps itself.

`lyon_tessellation` is not an automatic runtime fallback. It remains the first documented replacement
candidate if the admitted version later fails a gate, but its `f32` path model and generated-vertex
output do not satisfy Baozi's source-corner adapter without substantially more machinery.

## Correcting the Package Identity

The planning shorthand “GeoRust earcut” referred to the right project but left the Cargo name
ambiguous:

| Package | Current upstream | Finding |
| --- | --- | --- |
| `earcut` 0.4.10 | [georust/earcut](https://github.com/georust/earcut) | GeoRust candidate evaluated and admitted. |
| `earcutr` 0.5.0 | [frewsxcv/earcutr](https://github.com/frewsxcv/earcutr) | Different ISC-licensed project; repository is archived and is not GeoRust-maintained. Rejected. |

The official crates.io API reported `earcut` 0.4.10 as the newest non-yanked release, published
2026-05-07. GitHub reported the GeoRust repository as non-archived with its latest release and push
on 2026-05-07. The `earcutr` repository reported archived; its last crates.io release was 2025-05-29.

## Primary Sources

- [`earcut` 0.4.10 package](https://crates.io/crates/earcut/0.4.10),
  [API](https://docs.rs/earcut/0.4.10/earcut/struct.Earcut.html),
  [manifest](https://github.com/georust/earcut/blob/a381ffb8cf93db0eaa62ca7dadd52b7985c7e8ce/Cargo.toml), and
  [implementation](https://github.com/georust/earcut/blob/a381ffb8cf93db0eaa62ca7dadd52b7985c7e8ce/src/lib.rs)
- [GeoRust `earcut` source commit](https://github.com/georust/earcut/commit/a381ffb8cf93db0eaa62ca7dadd52b7985c7e8ce)
- [`lyon_tessellation` 1.0.20 package](https://crates.io/crates/lyon_tessellation/1.0.20),
  [API](https://docs.rs/lyon_tessellation/1.0.20/lyon_tessellation/), and
  [repository](https://github.com/nical/lyon)
- [`i_triangle` 0.45.1 package](https://crates.io/crates/i_triangle/0.45.1) and
  [repository](https://github.com/iShape-Rust/iTriangle)
- [`earcutr` 0.5.0 package](https://crates.io/crates/earcutr/0.5.0) and
  [archived repository](https://github.com/frewsxcv/earcutr)

Package manifests and source were read from the exact crates.io artifacts downloaded by Cargo.
Maintenance dates and archive state were queried from the official crates.io and GitHub APIs. The
general web-search endpoint returned HTTP 502 twice; no secondary search result was used as evidence.

## Candidate Comparison

| Gate | `earcut` 0.4.10 | `lyon_tessellation` 1.0.20 | `i_triangle` 0.45.1 | Local algorithm |
| --- | --- | --- | --- | --- |
| Coordinate API | Generic `Float`; `f64` spike compiled | Path and fill API uses `f32` | Accepts `f32`/`f64`, maps through integer core | Whatever Baozi builds |
| Output identity | Indices into input ring | Generated vertex buffer plus indices | Reordered/generated point buffer plus indices | Can be designed for source indices |
| Error API | `()`; documented panics for invalid holes/count | `Result<(), TessellationError>` plus assertions | Primarily value-returning; validation may repair | Baozi-owned `Result` |
| Invalid geometry | Must be rejected before call | Graphics fill semantics may split intersections | Validation repairs/simplifies intersections | Full burden on Baozi |
| License and notices | Package declares MIT OR Apache-2.0; artifact bundles `LICENSE-ISC` for Mapbox-derived portions | MIT OR Apache-2.0 | MIT OR Apache-2.0 | N/A |
| Explicit `rust-version` | None; edition 2024 implies Rust 1.85 language floor | None; edition 2018 | None; transitive `i_overlay` declares 1.88 | Baozi MSRV |
| Rust 1.95 check | Pass | Pass | Pass | N/A |
| WASM check | Pass | Pass | Pass | Must be built and maintained |
| Normal dependency graph | `num-traits`/`libm` | lyon path/geom, euclid, arrayvec, numeric helpers | overlay/tree/sort/shape stack | No dependency; largest algorithm burden |
| Maintenance at capture | Non-archived; release 2026-05-07 | Non-archived; crate release 2026-03-21, repo push 2026-05-03 | Non-archived; release 2026-06-24 | Baozi team only |
| Decision | Chosen behind adapter | Replacement candidate, not linked fallback | Rejected for source-preserving mapping | Rejected |

`earcut` is `#![no_std]` and has no FFI, but its 0.4.10 implementation contains internal `unsafe`
pointer access for indexed nodes. “Pure Rust” in this decision does not mean “unsafe-free.” This is a
dependency-audit and sanitizer risk, not permission for unsafe code in Baozi.

## Reproducible Spike

The spike was created outside the repository at
`%TEMP%/baozi-triangulation-admission-20260710`; it did not modify the workspace or production
manifests. The initial print-only harness produced the observations below. During U2 review, those
same fixtures and backend calls were hardened into the fail-closed assertions preserved in Appendix
A; the captured debug and release observations satisfy those predicates. Appendix A is the canonical
rerun source. Its manifest used exact direct versions:

```toml
[package]
name = "baozi-triangulation-admission"
version = "0.0.0"
edition = "2024"
publish = false
license = "MIT OR Apache-2.0"

[dependencies]
earcut = "=0.4.10"
i_triangle = "=0.45.1"
lyon_tessellation = "=1.0.20"
```

Environment:

```text
host: x86_64-pc-windows-msvc
Baozi MSRV toolchain used: rustc 1.95.0 (59807616e 2026-04-14)
Cargo used: cargo 1.95.0 (f2d3ce0bd 2026-03-21)
cargo-deny used: cargo-deny 0.19.9
active target check: wasm32-unknown-unknown
```

Commands:

```powershell
$baoziDenyConfig = (Resolve-Path 'deny.toml').Path
$spikeRoot = Join-Path $env:TEMP 'baozi-triangulation-admission-20260710'
$spikeManifest = Join-Path $spikeRoot 'Cargo.toml'
New-Item -ItemType Directory -Force -Path (Join-Path $spikeRoot 'src') | Out-Null
# Materialize Appendix A and the captured Appendix B lockfile under $spikeRoot.
Set-Location -LiteralPath $spikeRoot
cargo +1.95.0 run --locked
cargo +1.95.0 run --release --locked
cargo +1.95.0 check --target wasm32-unknown-unknown --locked
$spikeMetadata = cargo +1.95.0 metadata --format-version 1 --locked | ConvertFrom-Json
$earcutPackage = @($spikeMetadata.packages | Where-Object {
    $_.name -eq 'earcut' -and $_.version -eq '0.4.10'
})
if ($earcutPackage.Count -ne 1) {
    throw "expected exactly one earcut 0.4.10 package, found $($earcutPackage.Count)"
}
$earcutRoot = Split-Path -Parent $earcutPackage[0].manifest_path
foreach ($requiredNotice in @('LICENSE-MIT', 'LICENSE-APACHE', 'LICENSE-ISC')) {
    if (-not (Test-Path -LiteralPath (Join-Path $earcutRoot $requiredNotice))) {
        throw "earcut 0.4.10 is missing required notice $requiredNotice"
    }
}
cargo +1.95.0 tree -p earcut --locked
cargo +1.95.0 tree -p lyon_tessellation --locked
cargo +1.95.0 tree -p i_triangle --locked
cargo deny --manifest-path $spikeManifest --locked check --config $baoziDenyConfig
```

The initial capture created Appendix B with `cargo +1.95.0 generate-lockfile`. Exact reruns copy that
lockfile and use the commands above. A dependency-drift audit may deliberately regenerate it and
compare the resulting tree and checksums with this report.

The resolved normal dependency roots were:

```text
earcut 0.4.10 -> num-traits 0.2.19 -> libm 0.2.16 (+ autocfg 1.5.1 build)
lyon_tessellation 1.0.20 -> lyon_path 1.0.19, lyon_geom 1.0.19,
  euclid 0.22.14, arrayvec 0.7.8, float_next_after 1.0.0, num-traits/libm
i_triangle 0.45.1 -> i_overlay 7.0.2, i_tree 0.19.0, i_key_sort 0.10.3,
  i_float 3.0.0, i_shape 3.0.0, libm 0.2.16
```

The final dependency-policy command reported:

```text
advisories ok, bans ok, licenses ok, sources ok
```

The spike's own synthetic package initially omitted a license and was rejected as unlicensed. After
declaring `MIT OR Apache-2.0` for the synthetic root, the unchanged candidate graph passed. This was
an expected harness correction, not a dependency exception. `cargo deny` evaluates the package SPDX
expression but does not prove preservation of the additional Mapbox-derived ISC notice, so the
metadata block above independently requires all three exact-artifact license files. U7 source/package
checks and U9 release packaging must continue to preserve `LICENSE-ISC`; passing `cargo deny` alone
cannot satisfy that distribution gate.

### Fixture Matrix

All candidates received the same outer-ring coordinates where their APIs allowed it:

| Case | Coordinates or condition |
| --- | --- |
| Concave CCW | `(0,0) (3,0) (3,3) (1.5,1) (0,3)` |
| Concave CW | Exact reverse of the CCW ring |
| Consecutive duplicate | Square with the `(3,0)` corner repeated |
| Collinear boundary | Square with `(1.5,0)` on the bottom edge |
| Fully collinear | Four points on the X axis |
| Self-intersection | Bow-tie `(0,0) (3,3) (0,3) (3,0)` |
| Non-finite | One coordinate replaced by NaN or positive infinity |
| Invalid holes (`earcut`) | First hole offset out of range; descending offsets |

The harness uses `catch_unwind` to observe panic behavior and executable assertions to enforce every
earcut admission observation: in-range and deterministic indices, valid-ring triangle counts and
areas, retained collinear corners, invalid-input successful-looking output, and malformed-hole
panics. A drift exits non-zero before printing the final `admission_gate=pass` marker. Debug and
release are both required because assertions and float casts can differ by profile. The rejected
candidate output remains comparative evidence rather than an admission oracle.

## Observed Results

### GeoRust `earcut` 0.4.10

The exact API under test was `Earcut::<f64>::earcut(..., &[], &mut Vec<u32>)`.

| Case | Debug and release observation | Admission consequence |
| --- | --- | --- |
| Concave CCW | 3 triangles; source area `+6`, triangle area `+6` | Correct valid-ring coverage. |
| Concave CW | 3 triangles; source area `-6`, triangle area `+6` | Backend normalizes winding; Baozi must flip each triangle for CW source. |
| Consecutive duplicate | 2 triangles; area `9`; one duplicate source corner omitted | Baozi must reject or explicitly repair/report before call. |
| Collinear boundary | 3 triangles; area `9` | Supported, but Baozi still checks every triangle and total area. |
| Fully collinear | Empty output returned successfully | Degenerate area must fail before call. |
| Bow-tie | One triangle returned successfully; triangle area `4.5` while source signed area is `0` | Self-intersection must fail before call. |
| NaN | 2 triangles returned successfully; computed areas were NaN | Finite check must precede call. |
| Positive infinity | One triangle returned successfully; triangle area was infinite | Finite check must precede call. |
| Out-of-range hole offset | Panic from slice range | Never pass unvalidated hole offsets; current adapter passes no holes. |
| Descending hole offsets | Panic from invalid slice range | Same; known panic precondition is not an error API. |

All returned indices in the non-panicking cases were within the input ring. The crate's own public
`deviation` helper is not a replacement for Baozi output validation because it indexes the supplied
arrays and operates after malformed input has already reached the backend.

The source documents another panic when input exceeds its internal roughly 31-bit node index range.
Baozi's much smaller checked per-polygon cap makes that condition unreachable before allocation.

### `lyon_tessellation` 1.0.20

- Both concave windings returned `Ok(())`, five generated vertices, and three triangles.
- A consecutive duplicate produced four output vertices; source endpoint identity was not retained
  as direct indices.
- The bow-tie returned `Ok(())`, inserted a fifth intersection vertex, and emitted two triangles.
- NaN and infinity panicked during path construction in debug. In release, NaN returned
  `UnsupportedParamater(PositionIsNaN)`, while infinity returned `Ok(())` and two triangles.

The typed error is useful for graphics paths, but the debug/release difference and generated vertex
model prove that Baozi still needs identical validation and a more complex identity adapter. Its
`f32` geometry is also a regression from the admitted `f64` projection boundary.

### `i_triangle` 0.45.1

- Concave input succeeded but returned a reordered point buffer rather than source-ring indices.
- Duplicate and collinear boundary points were reduced from five to four output points.
- The bow-tie succeeded by inserting the intersection point twice and returned six output points.
- NaN panicked in debug but in release was silently mapped into a finite point and triangulated;
  positive infinity returned an empty result.

These are valuable repair semantics for a polygon overlay tool, but they conflict with Baozi's rule
that unsupported self-intersection fails and source corner/face data remains explicitly remappable.

## Admission Gates

| Gate | Required result | Evidence | Verdict |
| --- | --- | --- | --- |
| Actual project identity | GeoRust-owned published crate | crates.io manifest/repository and GitHub API | Pass for `earcut`; planning ambiguity corrected. |
| `f64` API | Compile and execute without narrowing | `Earcut::<f64>` debug/release runs | Pass. |
| License/source/notice policy | Repository allowlist, known source, and preserved bundled notice | Full spike graph `cargo deny`, exact-artifact `LICENSE-*` inspection, and U7/U9 preservation gates | Pass for dependency admission; source/package/release output remains blocked if `LICENSE-ISC` is absent. |
| Baozi MSRV | Rust 1.95 native build | Debug and release runs | Pass. |
| WASM target build | `wasm32-unknown-unknown` compile | Whole exact-version spike check | Pass. |
| Maintenance | Current, non-archived upstream | Official API state and 2026 release | Pass. |
| Valid concave behavior | Area-correct CW and CCW | Fixture results | Pass after adapter winding restoration. |
| Invalid-input behavior | No invalid success may escape | Spike proves invalid success/panics | Conditional: mandatory Baozi pre/post validation. |
| Source-domain mapping | Output traceable to source corners | Valid single-ring indices | Pass with Baozi working-ring/source-ring remap. |

## U7 Revalidation Rule

U7 should add the dependency only behind a private adapter and first resolve the lockfile. If the
resolved package is exactly 0.4.10, it must still rerun the MSRV check, WASM check, repository
dependency and notice policy, and complete adapter conformance suite; it does not need to reopen the
selection among libraries.

Any `earcut` version, feature-set, fork, source, or checksum drift reopens the complete backend
selection and admission decision before code is accepted. Transitive dependency drift reruns the
lockfile, policy, portability, and adapter-conformance gates. The full admission matrix is:

1. exact API and `f64` behavior;
2. license, source, advisories, and internal unsafe review;
3. Baozi MSRV plus WASM/WASI targets;
4. maintenance/archive status;
5. debug/release invalid-input and panic matrix; and
6. CW/CCW area, deterministic indices, source mapping, and output validation.

No backend failure automatically invokes lyon or another tessellator. A replacement is a reviewed
dependency decision, not a hidden runtime branch.

## Appendix A: Complete Spike Source

Create `$spikeRoot/src`, place the manifest shown in Reproducible Spike at
`$spikeRoot/Cargo.toml`, and place this file at `$spikeRoot/src/main.rs` before running the recorded
commands:

```rust
use std::panic::{AssertUnwindSafe, catch_unwind};

use earcut::Earcut;
use i_triangle::float::triangulatable::Triangulatable;
use lyon_tessellation::{
    FillOptions, FillTessellator,
    geometry_builder::{VertexBuffers, simple_builder},
    math::{Point, point},
    path::Path,
};

#[derive(Clone, Copy)]
struct Case {
    name: &'static str,
    points: &'static [[f64; 2]],
}

const CASES: &[Case] = &[
    Case {
        name: "concave_ccw",
        points: &[
            [0.0, 0.0],
            [3.0, 0.0],
            [3.0, 3.0],
            [1.5, 1.0],
            [0.0, 3.0],
        ],
    },
    Case {
        name: "concave_cw",
        points: &[
            [0.0, 3.0],
            [1.5, 1.0],
            [3.0, 3.0],
            [3.0, 0.0],
            [0.0, 0.0],
        ],
    },
    Case {
        name: "consecutive_duplicate",
        points: &[
            [0.0, 0.0],
            [3.0, 0.0],
            [3.0, 0.0],
            [3.0, 3.0],
            [0.0, 3.0],
        ],
    },
    Case {
        name: "collinear_vertex",
        points: &[
            [0.0, 0.0],
            [1.5, 0.0],
            [3.0, 0.0],
            [3.0, 3.0],
            [0.0, 3.0],
        ],
    },
    Case {
        name: "all_collinear",
        points: &[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 0.0]],
    },
    Case {
        name: "self_intersecting_bowtie",
        points: &[[0.0, 0.0], [3.0, 3.0], [0.0, 3.0], [3.0, 0.0]],
    },
    Case {
        name: "nan",
        points: &[[0.0, 0.0], [3.0, 0.0], [f64::NAN, 3.0], [0.0, 3.0]],
    },
    Case {
        name: "positive_infinity",
        points: &[
            [0.0, 0.0],
            [3.0, 0.0],
            [f64::INFINITY, 3.0],
            [0.0, 3.0],
        ],
    },
];

fn signed_area(points: &[[f64; 2]]) -> f64 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum::<f64>()
        / 2.0
}

fn indexed_signed_area(points: &[[f64; 2]], indices: &[u32]) -> f64 {
    indices
        .chunks_exact(3)
        .map(|triangle| {
            let a = points[triangle[0] as usize];
            let b = points[triangle[1] as usize];
            let c = points[triangle[2] as usize];
            ((b[0] - a[0]) * (c[1] - a[1])
                - (b[1] - a[1]) * (c[0] - a[0]))
                / 2.0
        })
        .sum()
}

fn approximately_equal(left: f64, right: f64) -> bool {
    (left - right).abs() <= 1.0e-12
}

fn run_earcut(points: &[[f64; 2]]) -> (Vec<u32>, bool, f64) {
    let mut backend = Earcut::<f64>::new();
    let mut indices = Vec::<u32>::new();
    backend.earcut(points.iter().copied(), &[], &mut indices);
    let valid_indices = indices
        .iter()
        .all(|index| (*index as usize) < points.len());
    let triangles_area = if valid_indices {
        indexed_signed_area(points, &indices)
    } else {
        f64::NAN
    };
    (indices, valid_indices, triangles_area)
}

fn assert_earcut_observation(
    case: Case,
    indices: &[u32],
    valid_indices: bool,
    triangles_area: f64,
) {
    assert!(valid_indices, "{} returned an out-of-range index", case.name);
    let polygon_area = signed_area(case.points);
    let referenced_corner_count = (0..case.points.len())
        .filter(|corner| indices.contains(&(*corner as u32)))
        .count();

    match case.name {
        "concave_ccw" => {
            assert_eq!(indices.len(), (case.points.len() - 2) * 3);
            assert!(polygon_area > 0.0 && triangles_area > 0.0);
            assert!(approximately_equal(polygon_area, triangles_area));
            assert_eq!(referenced_corner_count, case.points.len());
        }
        "concave_cw" => {
            assert_eq!(indices.len(), (case.points.len() - 2) * 3);
            assert!(polygon_area < 0.0 && triangles_area > 0.0);
            assert!(approximately_equal(polygon_area.abs(), triangles_area));
            assert_eq!(referenced_corner_count, case.points.len());
        }
        "consecutive_duplicate" => {
            assert_eq!(indices.len(), 6);
            assert!(approximately_equal(polygon_area, triangles_area));
            assert!(referenced_corner_count < case.points.len());
        }
        "collinear_vertex" => {
            assert_eq!(indices.len(), (case.points.len() - 2) * 3);
            assert!(approximately_equal(polygon_area, triangles_area));
            assert_eq!(referenced_corner_count, case.points.len());
        }
        "all_collinear" => {
            assert!(indices.is_empty());
            assert_eq!(polygon_area, 0.0);
            assert_eq!(triangles_area, 0.0);
        }
        "self_intersecting_bowtie" => {
            assert!(!indices.is_empty());
            assert_eq!(polygon_area, 0.0);
            assert!(triangles_area.abs() > 0.0);
        }
        "nan" => {
            assert!(!indices.is_empty());
            assert!(polygon_area.is_nan());
            assert!(triangles_area.is_nan());
        }
        "positive_infinity" => {
            assert!(!indices.is_empty());
            assert!(!polygon_area.is_finite());
            assert!(!triangles_area.is_finite());
        }
        unknown => panic!("unclassified earcut fixture {unknown}"),
    }
}

fn earcut_case(case: Case) {
    let result = catch_unwind(AssertUnwindSafe(|| run_earcut(case.points)));
    let Ok((indices, valid_indices, triangles_area)) = result else {
        panic!("earcut unexpectedly panicked for {}", case.name);
    };
    let (repeated_indices, repeated_valid_indices, repeated_area) = run_earcut(case.points);
    assert_eq!(indices, repeated_indices, "{} was nondeterministic", case.name);
    assert_eq!(valid_indices, repeated_valid_indices);
    assert_eq!(triangles_area.to_bits(), repeated_area.to_bits());
    assert_earcut_observation(case, &indices, valid_indices, triangles_area);

    println!(
        "earcut case={} status=ok polygon_area={:?} triangle_area={:?} valid_indices={} indices={:?}",
        case.name,
        signed_area(case.points),
        triangles_area,
        valid_indices,
        indices
    );
}

fn earcut_bad_holes() {
    let points = [[0.0, 0.0], [3.0, 0.0], [3.0, 3.0], [0.0, 3.0]];
    for (name, holes) in [
        ("out_of_range_hole", vec![5_u32]),
        ("descending_holes", vec![3, 2]),
    ] {
        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut backend = Earcut::<f64>::new();
            let mut indices = Vec::<u32>::new();
            backend.earcut(points, &holes, &mut indices);
            indices
        }));
        assert!(result.is_err(), "earcut case={name} unexpectedly succeeded");
        println!("earcut case={name} status=panic");
    }
}

fn lyon_case(case: Case) {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let mut path_builder = Path::builder();
        let first = case.points[0];
        path_builder.begin(point(first[0] as f32, first[1] as f32));
        for p in &case.points[1..] {
            path_builder.line_to(point(p[0] as f32, p[1] as f32));
        }
        path_builder.end(true);
        let path = path_builder.build();

        let mut buffers: VertexBuffers<Point, u16> = VertexBuffers::new();
        let tessellation = {
            let mut output = simple_builder(&mut buffers);
            FillTessellator::new().tessellate_path(
                &path,
                &FillOptions::default(),
                &mut output,
            )
        };
        (tessellation, buffers)
    }));

    match result {
        Ok((status, buffers)) => println!(
            "lyon case={} status={:?} source_vertices={} output_vertices={} indices={:?}",
            case.name,
            status,
            case.points.len(),
            buffers.vertices.len(),
            buffers.indices
        ),
        Err(_) => println!("lyon case={} status=panic", case.name),
    }
}

fn i_triangle_case(case: Case) {
    let result = catch_unwind(AssertUnwindSafe(|| {
        case.points.triangulate().to_triangulation::<u32>()
    }));

    match result {
        Ok(mesh) => println!(
            "i_triangle case={} status=ok source_vertices={} output_vertices={} indices={:?} points={:?}",
            case.name,
            case.points.len(),
            mesh.points.len(),
            mesh.indices,
            mesh.points
        ),
        Err(_) => println!("i_triangle case={} status=panic", case.name),
    }
}

fn main() {
    for &case in CASES {
        earcut_case(case);
    }
    earcut_bad_holes();
    for &case in CASES {
        lyon_case(case);
    }
    for &case in CASES {
        i_triangle_case(case);
    }
    println!("admission_gate=pass backend=earcut version=0.4.10");
}
```

## Appendix B: Captured Cargo.lock

The tested lockfile SHA-256 is
`9ec6962d7ecb14901a1089a20c2bc15a4c85f0310147851b3a671372df015c0f`.

```toml
# This file is automatically @generated by Cargo.
# It is not intended for manual editing.
version = 4

[[package]]
name = "arrayvec"
version = "0.7.8"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "d3fb67a6e08acf24fdeccbac2cb6ac4305825bd1f117462e0e6f2f193345ad56"

[[package]]
name = "autocfg"
version = "1.5.1"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "f2032f911046de80f0a198e0901378627c33f59ea0ac00e363d481118bd70a53"

[[package]]
name = "baozi-triangulation-admission"
version = "0.0.0"
dependencies = [
 "earcut",
 "i_triangle",
 "lyon_tessellation",
]

[[package]]
name = "earcut"
version = "0.4.10"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "7d71866fffb1c6d539cc36ef323109416cdc5686e17df0252d98c5e83641efc8"
dependencies = [
 "num-traits",
]

[[package]]
name = "euclid"
version = "0.22.14"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "f1a05365e3b1c6d1650318537c7460c6923f1abdd272ad6842baa2b509957a06"
dependencies = [
 "num-traits",
]

[[package]]
name = "float_next_after"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "8bf7cc16383c4b8d58b9905a8509f02926ce3058053c056376248d958c9df1e8"

[[package]]
name = "i_float"
version = "3.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "519b2780a1187eeed09145e5d168863c87f0d704abe84084206ebd897f99c6dc"
dependencies = [
 "libm",
]

[[package]]
name = "i_key_sort"
version = "0.10.3"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "d73d122b937fca067feb0ad74f62388920272b27c356d4df2d0cfdd59e044cf0"

[[package]]
name = "i_overlay"
version = "7.0.2"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "bf284de04038af713dad8b4a88d2bc97f1c8c0a87fc1f43e28e3b965d0136d9e"
dependencies = [
 "i_float",
 "i_key_sort",
 "i_shape",
 "i_tree",
]

[[package]]
name = "i_shape"
version = "3.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "90b2ee7bae072887c579a2f2fd2a7fa1c29cceecfae1998ef940061c5570afff"
dependencies = [
 "i_float",
]

[[package]]
name = "i_tree"
version = "0.19.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "1e9d4a992a9fe83130f41ceacceac3bb116a4355dfc9c8d6ecea4f1e4b4c6caf"

[[package]]
name = "i_triangle"
version = "0.45.1"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "1a22b562e49071a682109194226ceecd76c86537aeafb8d8831ecf60cfc2ab99"
dependencies = [
 "i_key_sort",
 "i_overlay",
 "i_tree",
]

[[package]]
name = "libm"
version = "0.2.16"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "b6d2cec3eae94f9f509c767b45932f1ada8350c4bdb85af2fcab4a3c14807981"

[[package]]
name = "lyon_geom"
version = "1.0.19"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "4336502e29e32af93cf2dad2214ed6003c17ceb5bd499df77b1de663b9042b92"
dependencies = [
 "arrayvec",
 "euclid",
 "num-traits",
]

[[package]]
name = "lyon_path"
version = "1.0.19"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "5c463f9c428b7fc5ec885dcd39ce4aa61e29111d0e33483f6f98c74e89d8621e"
dependencies = [
 "lyon_geom",
 "num-traits",
]

[[package]]
name = "lyon_tessellation"
version = "1.0.20"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "8e43b7e44161571868f5c931d12583592c223c5583eef86b08aa02b7048a3552"
dependencies = [
 "float_next_after",
 "lyon_path",
 "num-traits",
]

[[package]]
name = "num-traits"
version = "0.2.19"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "071dfc062690e90b734c0b2273ce72ad0ffa95f0c74596bc250dcfd960262841"
dependencies = [
 "autocfg",
 "libm",
]
```
