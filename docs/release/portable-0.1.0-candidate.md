# Portable 0.1.0 candidate qualification

This is a finite record for the first canonical portable candidate. Local qualification is
complete for the scopes below under the [reviewed maturation plan](../plans/2026-11-02-001-refactor-first-release-maturation-plan.md).
The [execution record](../reviews/2026-11-02-first-release-maturation.md) owns source identities,
commands and observed results. This document does not authorize registry operations.

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

## Separate publication work

The [release policy](../contributing/release-policy.md) and ADR 0006 own the future release train,
including the approved registry expected-state record, ownership/authentication preflight,
dependency upload order and a fresh registry-only consumer. The existing registry `baozi 0.0.1`
is a placeholder; verify a real replacement before any separately authorized yank. No upload,
registry yank, push, PR or native promotion is part of local qualification.
