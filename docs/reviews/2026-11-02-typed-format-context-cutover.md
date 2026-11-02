# Typed Format and Parse Context Cutover

Status: integrated source qualification complete. This receipt is not a release or performance result.
Base: `4abc3998605f865bf6395347d9048dae060102c5`.
Implementation: `89f4e0bba832a43be55ce60159cc3b1790040820`.

This implements the atomic P3 import/P4 interface portion of the
[operation architecture continuation](../plans/2026-11-02-1700-perf-import-operation-architecture-plan.md).
The original fifteen-unit pre-release goal remains active. P5, P6, full Linux B1/heap observations
and final U14 qualification remain required. Publication, push/PR and native promotion are outside
this slice.

## Resulting Interface

- `Format` associates one concrete `Options` type with bounded default/layer compilation and
  `parse(&Self::Options, &mut ParseContext) -> ParseResult<()>`. Private registry adapters erase the
  format and its compiled options together. Wire schema/version/layer validation remains before IO;
  public runtime keys, option codecs and parse-time downcasts are removed.
- `ImportPlan` retains the registry, compiled invocations and shared storage-account factory. It
  executes its own byte/asset/resolver imports. There is no arbitrary importer/plan pairing or
  compatibility alias for the retired request type.
- Outer `ParseContext` alone submits a static builder or a complete authoring batch. Higher-ranked
  execution regions lend the actual work, storage and control views. Bound work groups and permits
  cannot escape. Metered containers, dependency handles, text cursors and source builders retain
  their genuine input/session storage lifetimes and receive execution explicitly when needed.
- The startup-admitted state slot is an ordinary exclusive value. The whole-owner `RefCell` and
  parser container host reacquisition route are removed. The sole local work owner still supplies
  parent/child channels, admission and immediate physical completion; no second accounting engine
  or deferred completion stream is introduced.
- Scene candidates remain private to the session. Missing/duplicate submission, a swallowed failure,
  error/panic after submission, registration poison or final cancellation cannot publish a Scene.
  Core retains construction-brand, proof, generation and prepared-root/handoff checks; public
  `SceneDraft` delivery is removed.

STL, OBJ/MTL, PLY, glTF, public examples, custom importers, excluded consumers, fuzz callers and
current ADRs/guides use this interface. The glTF route stages actual model and descriptor tables
outside execution borrows, releases decoder/preflight storage before canonical authoring, and
submits through the same complete-batch implementation.

## Control and Storage Corrections

Polygon kernels now emit Begin/Complete/Checkpoint around actual bounded leaves, replacing empty
work callbacks. Semantic errors complete the leaf that ran; unwind emits no synthetic completion.
The existing permit Drop path resolves pending ownership after errors/unwind without polling user
controls. Successful unfinished ownership remains a sticky contract failure.

A real OBJ cancellation test exposed terminal evidence with an already selected Ledger cause and
a later signal cancellation. Handoff now retains that supplementary control observation rather
than treating it as inconsistent ownership. Public failure selection follows the sole ledger class;
`terminal_control_cause` retains the later interruption. Freeze-first control still supplies its
exact origin, and authoritative Scene-validation errors retain their existing exception.

The hard aggregate `TotalAssetBytes` ceiling now remains terminal before optional cache/loss/
diagnostic work can replace its cause. Authorized acquisition-local dependency byte/work omissions
remain separate. Tests assert the rejected attempt, actual consumed prefix and absence of optional
loss evidence after the hard aggregate failure.

Destructor tests observe actual logical reservations rather than physical retained bytes. Independent
review rejected an initial escaping weak observation handle: it could retain a ledger allocation
and race exclusive terminal detach. That implementation is removed. The replacement is a borrowed,
non-Send/non-Sync storage view tied to the actual execution lifetime and requires no allocation. It owns neither
`Arc` nor `Weak`; positive terminal-detach tests and lifetime/Send/Sync compile-fail cases cover the
corrected boundary. Physical account cleanup remains independently asserted.

## Verification

All Cargo work is serialized with Rust 1.95.0, locked offline dependencies and one build job.
Results below describe the integrated source above. The final test-only facade correction
was followed by fresh full workspace nextest, strict Clippy and format checks.

| Check | Observed result |
| --- | --- |
| Import/OBJ/glTF refreshed nextest | 1,106/1,106; `2a98c552-bdac-4d80-846d-8d1fbc193b42` |
| Postprocess, including real polygon leaves and pending cleanup | 257/257, two existing manual skips; `5fac3a68-058e-44e2-800b-5f130029e91b` |
| Workspace all-target/all-feature compile | Passed after migrating three missed facade examples |
| Workspace strict all-target/all-feature Clippy | Passed; no new lint suppression |
| Workspace doctests | 63/63 passed, including typed options and execution/storage lifetime boundaries |
| Workspace docs with warnings denied | Passed after quoting an array literal in a core doc comment |
| Independent assets consumer | 120/120, one existing manual skip; `4bed4ec6-68c7-4252-9cf8-fa911d021201`; strict Clippy/fmt passed |
| Independent glTF conformance consumer | 8/8, one existing manual skip; `7ffe66b5-01d6-4494-9337-8194cde54231`; strict Clippy/fmt passed |
| Renderer CPU consumer | 37/37; `6130e856-4126-483d-9102-8d8622170100`; strict Clippy/fmt passed; 25 manual GPU cases not executed here |
| Public scene consumer | All four formats and custom/sidecar/evidence workflows passed |
| Fuzz seed regression suite | 16/16; `a35e69aa-468e-4dab-9696-3a94c695c1f9`; strict Clippy/fmt passed |
| Workspace final nextest | 3,221/3,221, six existing manual skips; `aa96f67c-2942-42b5-bfef-8425e37e337b` |

The paired-live-context table checks eleven foreign-capability cases, including zero work, zero
lanes, empty/no-op storage growth, source records and exhausted cursors. Original issuers remain
usable; rejection precedes callback/admission/mutation and both accounts release correctly.
Additional candidate/format-error tests distinguish no cancellation from first Finalize observer
and signal cancellation. Exact text/record/UTF-8 prefixes replace obsolete callback-count schedules.

The independent scene fixture's byte-component joint attribute now declares its required four-byte
stride under the pinned glTF alignment rule. Its binary bytes and semantic assertions are unchanged;
this corrects the fixture rather than relaxing production validation.

The first complete workspace run exposed one further facade expectation that combined a hard
aggregate-byte ceiling with an optional local dependency limit. The corrected test observes one
dependency reference and two opens (primary plus the first sidecar), the exact rejected aggregate
attempt and primary-only consumed bytes, and no optional evidence. A separate positive test keeps
both repeated local omissions successful when the total budget permits them. No production
behavior was changed to satisfy either test. The failed run and first corrected-assertion run
remain in the archive rather than being relabeled as passing qualification.

Independent source review covered typed registration/options/plans, core/context ownership,
glTF staged authoring, and late-failure/storage corrections. Its two actionable observation-lifetime
findings are resolved by the replacement above. Review is not a substitute for runtime qualification.

## Source and Evidence

`target/review/p4-tested-source.json` retains the 203 changed source/test/current-document file
hashes, the base and implementation revision. `target/review/p4-source.tar` is the exact committed
source archive, SHA-256 `85208964e94171e78575367c52e5af2fd0954ba705393c4b6921f7f77507c7ce`.
The verification archive `target/review/p4-context-evidence.zip` retains 97 command records,
logs, source inventory, patch and independent reviews, SHA-256
`966cba5e0b1909f1e21463438aac778dc441d84d91ae1137426f854c87412442`.

## Remaining Work

The earlier ordinary Windows foundation measurement still records a grid-import regression.
Remeasure the unchanged six lanes on a committed integrated source before accepting P3 performance.
Neither typed dispatch nor fewer wrappers alone establishes a speedup. Full Windows/Linux comparison,
separate heap observations, per-format control probes and final package/portable/sanitizer/GPU
qualification remain governed by the original plans. The separately prepared Assimp control probe
is outside this interface commit and supplies no timing result here.
