# Portable 0.1.0 candidate qualification

This is a finite record for the first canonical portable candidate. Original maturation
qualification is complete for its historical scopes below under the [reviewed maturation plan](../plans/2026-11-02-001-refactor-first-release-maturation-plan.md).
The [execution record](../reviews/2026-11-02-first-release-maturation.md) owns source identities,
commands and observed results. This document does not authorize registry operations.

The original verification table retains its `6fb9112` source. The later evolution and architecture
changes have their own [dated qualification](#2026-11-03-evolution-qualification); old counts and
artifact identities are not transferred to that source.

## Release unit and profile

The release unit contains exactly ten packages, all version `0.1.0`: `baozi-operation`,
`baozi-io`, `baozi-core`, `baozi-import`, `baozi-format-stl`, `baozi-format-obj`,
`baozi-postprocess`, `baozi-format-ply`, `baozi-format-gltf` and `baozi`.
There is no maintained glTF decoder package in the production graph.

| Profile | Facade selection | Candidate scope |
| --- | --- | --- |
| [STL](../formats/stl.md) | Default or `format-stl` | Documented ASCII/binary triangle and color profile |
| [OBJ/MTL](../formats/obj.md) | Default or `format-obj` | Documented static geometry, material/sidecar and source-loss policies |
| [PLY](../formats/ply.md) | Opt-in `format-ply` | ASCII, little-/big-endian geometry, declared F32/F64 precision, documented colors/custom scalars and metadata |
| [glTF/GLB](../formats/gltf.md) | Opt-in `format-gltf` | Selected-scene PBR/unlit, seven primitive modes, sparse accessors, complete influences, skin/morph/TRS curves and exact selected extras within the named restrictions |

`all-formats` selects all four. `default-features = false` selects no built-in format.
The complete public facade feature union is portable and requires Rust 1.95 plus the standard
library. Native filesystem/repository packages remain excluded, unpublished candidates, with
no release-registered native target. Encoded image retention does not include pixel decoding.

## PLY and glTF eligibility

| Gate | PLY | glTF/GLB |
| --- | --- | --- |
| Source admission | Existing documented profile through the facade | Existing selected profile through the facade |
| Maturation requirements | Retained declared width, bounded metadata, derived numeric/query/public-consumer regressions | Owned metered document, selected semantics/default-material regressions, production fork removal |
| Local verification authority | U2 and U8 in the execution record | U3 and U8 in the execution record |
| Publication manifest | `publish = false` | `publish = false` |
| Managed declaration/maturity | `Unverified` / no assigned Baozi maturity | `Unverified` / no assigned Baozi maturity |

Local archive checks can complete with those publication flags still disabled. A later release
change must explicitly complete the documented registry gates and enable publication of these
supporting packages before the facade. This record does not promote either format to Beta/Stable,
claim full format conformance or substitute a build flag for executed semantics.

## Verification scope

The immutable implementation is `6fb9112cb75e1dc942f6d7e605aa9e7b23ec1559`; the production
closure is the ten packages above. The execution record retains the source archive digest,
commands, nextest run IDs, actual artifact digests and practical limits.

| Local gate | Result and scope |
| --- | --- |
| Windows x86_64 MSVC, Rust 1.95.0 | 2,855 workspace tests, 74 independent assets, 58 doctests and 12 fuzz seeds pass; strict Clippy and formatting pass |
| Linux x86_64 GNU under WSL, Rust 1.95.0 | 2,855 workspace tests, 74 independent assets and 12 fuzz seeds pass |
| Unchanged archives | Ten packages; all 12 feature cases build and execute on Windows 1.95.0/debug and Linux installed stable 1.97.1/release; version-only roots, no root patches or rewritten artifact manifests |
| Browser wasm32 | Empty/all source consumers compile on Rust 1.95.0; no browser execution |
| WASI preview 1 | Empty/all source consumers and 57 core snapshot tests execute in release on Wasmtime 50.0.0, including fixed digests, F64 bits and material assignment |
| Linux AddressSanitizer | Six malformed glTF tests pass; glTF and edit-lifecycle targets each complete 256-run smoke, 120-second exploration and 30-second saved-corpus resume without a failing artifact |

Workspace runs skip four existing manual tests; independent assets skip one. Browser compilation,
local WSL execution and offline standard Cargo source replacement keep their exact scope.
Earlier revision evidence and configured hosted workflows retain their own source identities.
No current Apple ARM64 execution or other unexecuted-host qualification is claimed here.

## 2026-11-03 evolution qualification

The later [pre-release evolution plan](../plans/2026-11-02-0923-refactor-pre-release-evolution-plan.md)
and [architecture continuation](../plans/2026-11-02-1700-perf-import-operation-architecture-plan.md)
retain the same ten-package `0.1.0` train. Baozi-owned draft revision families are canonical v1;
this is not Cargo SemVer `1.0.0` or a rewrite of historical evidence.

The [final integrated receipt](../reviews/2026-11-03-final-pre-release-qualification.md) owns
execution at `088d7176d5982d1ae9e5045c0250dc6f608f66c1`, with production runtime
`52446ec8511b7dfba2e5352ce6941558d85e0ae2`. Final integrated execution is complete; that receipt
distinguishes passing scopes from the preserved initial failures and their dispositions. It does not
reuse the older archive or sanitizer success as evidence for the new closure.

The documented selected glTF profile additionally maps emissive strength, clearcoat, sheen,
punctual lights and admitted `KHR_mesh_quantization`, with complete selected resource closure.
Typed factor groups, format options, borrowed parse contexts, private scene publication and
cooperative import controls replace the retired interfaces. The direct single-format consumer
uses public importer APIs; the private owned decoder is not a public document crate or a gltf-rs
API replacement. The excluded wgpu consumer's named Vulkan readbacks do not add graphics or pixel
codecs to the public facade dependency closure.

| Dated local gate | Actual result |
| --- | --- |
| Windows/Linux Rust 1.95.0 host | Each: 3,278 workspace, 120 assets, eight direct glTF, 37 renderer CPU, sixteen fuzz seeds and 63 doctests; exact exclusions in the receipt |
| Strict quality checks | Windows workspace/independent all-target/all-feature Clippy, fmt and public docs passed |
| Unchanged archives | Twelve facade cases, direct glTF and native baseline execute on Windows 1.95.0/debug and Linux stable 1.97.1/release; optional capture/helper checks compile only; version-only roots without root patches |
| Browser/WASI | Browser empty/all compile only; WASI empty/all execute, with 58 snapshots and three glTF issue tests under Wasmtime 50.0.0 |
| Named device | 25 actual Vulkan readbacks on RTX 4090, vendor 4318/device 9860, NVIDIA 621.01 |
| Scoped sanitizer/fuzz | 242 ASan tests pass; two existing manual diagnostics excluded; both fuzz targets complete smoke, exploration and saved-corpus resume with zero failing artifacts |

The final source/log/package evidence archive has 1,763 individually verified members,
SHA-256 `95acd86abc07579528474dfc03b09dc5ec53bf42d7eb87525266e000e1819ade`.
The final receipt identifies the fourteen-archive extended closure per host; only the ten
portable packages form the release unit. Historic `6fb9112` counts remain in their original table.

The [Windows](../reviews/2026-11-03-windows-final-comparison.md) and
[Linux](../reviews/2026-11-03-linux-b1-and-heap-comparison.md) comparison receipts retain
110 ordinary lanes each, plus separate Linux heap observations. Measured improvements do not
establish Assimp parity. Fifty-seven Linux RSS budget breaches remain explicitly accepted costs
above their original limits; no all-budget-pass claim follows. The [updated development route](../research/2026-11-03-baozi-assimp-next-steps.md)
records the remaining workflow, performance and distribution choices.

This update does not enable PLY/glTF publication, assign managed format maturity or promote any
native candidate target. Browser compilation, actual WASI execution and named GPU readback remain
separate evidence scopes. Registry-only installation still belongs to the publication work below.

## Separate publication work

The [release policy](../contributing/release-policy.md) and ADR 0006 own the future release train,
including the approved registry expected-state record, ownership/authentication preflight,
dependency upload order and a fresh registry-only consumer. The existing registry `baozi 0.0.1`
is a placeholder; verify a real replacement before any separately authorized yank. No upload,
registry yank, push, PR or native promotion is part of local qualification.
