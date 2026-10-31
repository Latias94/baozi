# Source-Object Metadata and glTF Extras — 2026-10-31

## Scope and Decision

This increment addresses the selected `/asset/extras` rejection found by the
[previous real-asset integration](2026-10-31-real-asset-consumer.md). It preserves descriptive
metadata without inventing canonical entities or changing quaternion admission. The unchanged
`MorphPrimitivesTest` becomes a successful fixture; `SimpleSkin` remains the one numeric-policy
rejection. No native backend registration, publication, automatic numeric recovery, or general
asset serializer is introduced.

Metadata now distinguishes canonical entity owners from retained source-object owners. A source
descriptor references the existing source identity inventory. Document, source scene, buffer/view/
accessor, and animation-sampler properties can therefore survive independently of their canonical
consumers. The entity provenance coverage directory keeps its original membership. Metadata
scratch counts, cursors, last-input rows, storage admission, and work include both owner domains.
Central validation checks source membership, canonical domain/owner/key order, and exact pool
coverage. Entity descriptors follow edits and deletion; source descriptors retain source ownership.

Raw descriptive JSON uses the existing bounded byte value and `application/json`. It is not
converted to a second recursive metadata IR. The glTF scanner records the exact value range,
including GLB container offsets and explicit null presence; the normalized model retains only
selected source IDs, site keys, and ranges over the session's immutable primary snapshot. Authoring
borrows that range after decoder teardown, and core materializes the admitted retained payload.
Five texture binding/transform sites remain distinct to avoid owner-local key collisions.

Core's borrowed metadata, source, resource, and locator readers project the retained pools directly.
Source/resource row lookup uses existing segment range ends rather than a new global directory or
cursor rescan; appended empty segments remain valid. Entity metadata tokens include generation.
The standalone application explicitly copies its glTF source-JSON subset before destroying Scene.
It retains the sanitized resource text, format, versioned locator, key, MIME, and payload bytes.

## Exact Source Evidence

The official Khronos sample corpus remains pinned at
`8f492390ddc7a2ae221a1369ef560589cf8c70fb`; all upstream bytes and attribution files are unchanged.
The [extras schema](https://github.com/KhronosGroup/glTF/blob/main/specification/2.0/schema/extras.schema.json)
permits any JSON type. Null, scalars, empty containers, escapes, whitespace, and number spelling are
preserved as bytes, while an absent property produces no metadata entry. Arbitrary data inside
extras is inert and does not authorize extension or canonical reference behavior.

`MorphPrimitivesTest.gltf` is 5,610 bytes, with a 5,609-byte root JSON object followed by a newline.
Its `/asset/extras` occupies `[98,245)`, exactly 147 bytes, with SHA256
`ad6279eee1280dad037dc16cc5c33b8ef5c4f0f896aa7021568b1ee9e564a494`.
The decoded title is `multiple_primitives`, author is `ft-lab`, and the literal license is
`CC BY-4.0 (https://creativecommons.org/licenses/by/4.0/)`.

The application checks those original bytes after both CCW and final-CW processing paths and
Scene destruction. It also checks both primitive draws (72 + 24 vertices), their distinct
materials, shared default morph weight `0.5`, the position delta's 20 cm maximum Y after unit
conversion, and exact JPEG bytes. The corpus now has six successes and one explicit rejection.
Its complete 55-entry checksum manifest covers every fixture file except itself.

## Verification and Review

On Windows x86_64 with Rust 1.95.0:

- All-feature workspace nextest passes **2,389/2,389**, excluding `baozi-xtask`; the four existing
  manual storage pilots remain skipped. This includes all **218** glTF tests and the new core
  metadata regressions. Log: `target/review/source-extras-workspace-nextest-msrv.log`.
- The standalone application's all-feature nextest suite passes **10/10**.
  Log: `target/review/source-extras-assets-nextest-msrv.log`.
- Strict all-target/all-feature workspace and application Clippy, workspace/application format
  checks, and all **58 doctests** pass. Logs use `target/review/source-extras-` with suffixes
  `workspace-clippy-msrv.log`, `assets-clippy-msrv.log`, and `doctests-msrv.log`.
- Rustdoc builds the complete all-feature workspace with warnings denied and no dependency docs.
  Log: `target/review/source-extras-rustdoc-msrv.log`.
- The independent Scene consumer executes its empty, glTF-only, and complete public feature cases
  successfully, outside workspace feature unification.
  Log: `target/review/source-extras-consumer-features-msrv.log`.

New regression coverage includes all 20 metadata value variants and allocation-free borrowed
reads, scalar bit preservation, mixed-owner ordering/duplicates/bounds, exact work and byte
admission, and independent source-owner central validation. Edit regressions cover triangulation,
space conversion, source/resource lookup after empty segments, and Tangent deletion that retains
same-index source metadata. glTF regressions cover JSON/GLB spelling and presence, distinct nested
sites, shared animation samplers, selected source closure, and rejection before dependency IO when
selected extras exceed their existing limits.

Independent core ownership/accounting and glTF source/range/selection reviews found no confirmed
implementation defect. The core reviewer identified a test whose out-of-range source mutation
also broke descriptor order. It now mutates the last descriptor, first proves an in-range source
owner succeeds, then rejects both one-past-end and `u32::MAX` without an ordering violation.

Release observations remain separate from the debug correctness results above.
