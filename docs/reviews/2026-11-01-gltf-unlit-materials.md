---
status: in_progress
date: 2026-11-01
---

# Typed glTF unlit materials: execution review

This record covers the [unlit execution plan](../plans/2026-11-01-008-feat-gltf-unlit-materials-plan.md)
after `1ce311b`. It delivers candidate 7 of the
[asset workflow plan](../plans/2026-11-01-001-feat-asset-workflow-expansion-plan.md). Implementation
identity and integrated results are recorded after source freeze below.

## Behavior and decisions

The existing `ShadingModel::Unlit` is the canonical behavior selector. Each selected format-private
material contract now owns its shading model with its five existing texture contracts; the normalized
material row carries that closed value across backend teardown into canonical authoring. Public
`MaterialRef` already exposes shading, alpha, sidedness, factors and bindings. No core material table,
factor group, descriptor registry, public reader or snapshot layout is duplicated.

glTF Unlit evaluates MR base color and the BaseColor binding with vertex color and alpha coverage;
double-sided state remains explicit. Complete original MR/emissive factors, lighting bindings and
their full resource/provenance closure remain retained fallback source truth. Lighting-only textures
remain required when authored. Consumers choose active inputs from the shading model; the importer
does not erase fallback data, apply a material approximation or render pixels. The MR base-color
guarantee belongs to the glTF producer and does not constrain other formats' generic Unlit values.

The maintained backend's existing unlit feature permits required-name validation. Baozi's raw source
inventory independently checks selected object shape, `extensionsUsed`, material owner/site and
unsupported properties before dependency IO. The scanner recognizes escaped keys and records
unlit extras/extensions plus unknown direct properties with original key/member/value ranges.
Unknown values and extras remain opaque. Existing texture-transform ranges and behavior are retained.
Unlit extras use `org.baozi.gltf:unlit-extras` on the original material source identity. Wrong sites,
unknown direct properties and nested unsupported extension payloads cannot disappear through the
backend's empty Unlit structure. Empty nested extensions are allowed; malformed nested containers
fail the existing object-shape rule.

Unsupported required extensions fail globally. Unsupported retained payloads fail even when callers
allow `SourceSemanticLoss`: loss authority cannot define an unknown extension's fallback, references
or composition. Future fallback admission needs a schema-owned mapping and durable evidence first.
Unsupported optional payloads outside the selected closure remain scope omissions. An unsupported
optional declaration without a retained payload emits only
`gltf.extension_uninterpreted_declaration`, after selected-payload validation. This replaces
`gltf.extension_ignored` and its incorrect payload-omission claim, without a compatibility alias or
new semantic-loss category.

The independent application now copies material shading/alpha/sidedness and COLOR0 into its existing
owned layout. Color uses optional F64 RGBA values, exact F32 widening, RGB alpha one and retained
RGBA/absence. The complete color bits participate in the existing source-identity/tuple reuse key;
color seams and signed zero cannot be welded. This is a concrete consumer layout, not a renderer or
complete material exporter.

The normative source is the [pinned official Khronos extension](https://github.com/KhronosGroup/glTF/blob/59ecbdba704206bb656965ab4e202881806066f1/extensions/2.0/Khronos/KHR_materials_unlit/README.md)
and [object schema](https://github.com/KhronosGroup/glTF/blob/59ecbdba704206bb656965ab4e202881806066f1/extensions/2.0/Khronos/KHR_materials_unlit/schema/material.KHR_materials_unlit.schema.json).
Khronos permits additional extension-object properties with undefined interpretation; Baozi's
profile rejects unsupported behavioral properties with `gltf.unsupported_feature`, rather than
misclassifying them as schema-invalid. No Assimp implementation or fixture was copied.

## Review and verification

Four native Codex agents provided read-only material/policy review and disjoint scanner, importer
regression and consumer changes. The main thread owns production integration and serial Cargo.
The design review confirmed that independent factor retention and the existing public readers are
sufficient, identified the consumer's missing COLOR0 input and verified the backend required-name
gate. Integration review and revision-qualified results follow below.
The read-only integration pass found no confirmed production defect in raw payload/site handling,
selected-versus-omitted admission, normalized shading propagation or required-name validation.
The consumer's color extraction and reuse-key changes also passed its focused integration/unit run.

The 19 importer regressions cover optional/required/default and mixed materials; alpha/sidedness and
vertex colors; all fallback factors, slots and resources; missing/duplicate declarations; malformed
values, wrong sites, unknown properties and nested extensions; both loss-policy choices; declaration
diagnostics; omitted invalid material payloads; escaped JSON/GLB extras and source identity; exact
source metadata key admission before dependency IO; and snapshot differences with equal source
ranges and unchanged factors. The initial budget control counted all scene metadata instead of the
specific unlit extras entry. It now checks the exact key, source owner and JSON bytes; the corrected
budget and all other importer tests pass without weakening production validation.

| Executed check | Result | Run identity |
| --- | --- | --- |
| New scanner tests | 6/6 pass | `9f84fcf8-e009-4bb5-829c-1dee86a6c434` |
| Existing JSON admission and texture transforms | 19/19 pass | `6c523bf0-4c2a-486e-892b-d7fe224e23b4` |
| New importer tests | 19/19 pass | `f99fe974-a8d2-413c-aaa2-231affe69655` |
| New independent consumer cases and library units | 11/11 pass (5 integration, 6 units) | `eedcde52-3724-4208-a266-cc5178808b49` |
| Windows workspace, all features | Pending | Pending |
| Independent asset consumer, all features | Pending | Pending |
| Strict Clippy and formatting | Pending | Pending |

## Qualification boundary

Rust 1.95.0, locked/offline dependencies, one Cargo build job and the shared target directory are
used on x86_64-pc-windows-msvc. Tests use bounded self-authored fixtures. This slice does not add a
renderer, pixel validation, extension-reference schemas, format maturity promotion, package release,
sustained fuzzing, performance qualification or an animation runtime. Existing cancellation and
metering mechanisms are reused; new claims are limited to the executed checks recorded here.
