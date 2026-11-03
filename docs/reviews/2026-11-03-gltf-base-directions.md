# Selected glTF FLOAT Base-Direction Qualification

Date: 2026-11-03. Runtime source commit:
`0484dcb581869afef106f43721ffbb1589e1b631`. This closes the selected FLOAT admission
follow-up in the [normal standards research](../research/2026-11-03-normal-generation-standards.md)
and [execution plan](../plans/2026-11-03-1945-fix-gltf-base-directions-plan.md).
It is a correctness qualification, not a performance keep or release approval.

## Result and Contract

Selected FLOAT base NORMAL and TANGENT.xyz now require
`abs(((x*x + y*y) + z*z) - 1) <= 1e-4`, with each decoded F32 component widened to F64
before multiplication. This is Baozi's explicitly chosen admission tolerance, not a glTF
normative epsilon or the Khronos Validator's default. Some previously accepted finite
malformed assets now return `InvalidAccessor` with their accessor index and semantic reason.

The existing sample collector applies the predicate after logical sparse replacement and
existing finite/TANGENT.w checks, before storing or completing that sample. There is no second
traversal, normalization, output-bit change, allocation, new option or shared/core predicate.
The existing sample-work envelope, admission/completion vectors and polling remain intact.
Source-identity area-weighted Smooth, missing-normal consumer policy and benchmark tasks remain
unchanged. Quantized directions keep their decoded-magnitude policy; morph directions remain
signed displacements. This does not certify all accepted quantized vectors as normalized or
claim full glTF conformance.

Eight public regression tests cover dense/full-sparse directions with and without a base,
shadowed invalid dense values, explicit zero/nonunit/extreme F32 inputs, original bits and
signed zero, both signs of adjacent tolerance-boundary F32 values, finite/sign error priority,
FLOAT with quantization declared, morph zero/signed/nonunit values and exact failure/control
prefixes. A bad third NORMAL retains 3 vertices/5 attribute cells/1 stream; a bad third TANGENT
retains 3/8/2. Moving the same failure from row zero to row two retains exactly 64 additional
consumed WorkUnits. Invalid samples and stream-final suffixes are not completed.

The cancellation test uses the actual **256**-row loop interval and 257 point vertices.
At the row-256 poll, NORMAL retains 257 vertices/513 cells/1 stream; TANGENT retains
257/770/2. Valid and invalid counterparts cancel at the same poll with equal WorkUnits,
signal-owned cause, no completed primitive and zero remaining live/evidence/reservation or
committed-canonical state. The iterator may already have decoded the next sample; the test
proves cancellation before its direction validation, storage and completion.

## Windows Verification

Rust 1.95.0, locked/offline dependencies, one build job, the shared repository target and
serialized commands. Main exclusively ran Cargo. The three Rust files had identical hashes
before and after the serialized command sequence, and their committed Git blobs match those
qualified bytes.

| Check | Result |
| --- | --- |
| Complete glTF nextest suite | **525 passed**, zero skipped; `10f30333-98d4-4f0b-875f-cb3b318d9f58` |
| Maintained assets consumer, all features | **138 passed**, one existing manual skip; `257606e4-bc7e-4476-b4d8-014e3670e2ff` |
| glTF strict Clippy, all targets/features | Passed |
| Assets strict Clippy, all targets/features | Passed |
| glTF fmt | Passed |

This is Windows source qualification. It supplies no new Linux execution, sanitizer result,
ordinary benchmark or allocation measurement. The full performance plan remains active.

## Fixture Repairs and Retained Failures

Two existing decode/copy controls used all-zero binary payloads, including zero NORMALs.
Their setup now writes unit normal Z at F32 offsets 44, 56 and 68; positions, UVs, indices,
logical cardinality and buffer lengths retain their previous meaning. Base64 remains
262,144 encoded/196,608 decoded bytes; external buffers remain 131,072/262,144 bytes.
The original cancellation, attempted/consumed work, dependency/manifest bytes and cleanup
assertions remain. Independent source comparison checked their assertion suffixes unchanged.

| Preserved checkpoint | Actual result and resolution |
| --- | --- |
| Initial five-test red run `50f9e9e4-3445-46b8-841d-6b3ccd501409` | Three controls passed; two expected-rejection tests failed because malformed FLOAT directions were accepted. |
| First seven-test run `10d2f7e0-cc12-4ad4-b094-31fe693983d9` | Six passed; the cancellation oracle incorrectly used the last malformed-import poll, which could be an error-exit poll after two sample completions. |
| First complete glTF run `c36b659b-e0b2-4555-896b-b2b24f647b5f` | 522 passed, two legacy zero-normal fixtures failed. Corrected their payloads without weakening assertions. |
| Expanded focused run `2b0036f7-b8d4-4deb-b523-f880bc6197ab` | 15 passed; a 65-row test assumed a nonexistent row-64 poll and observed 130 cells versus 129 in its valid control. Corrected the test to the actual 256-row interval; production polling did not change. |

The mistaken 64-row advice and its explicit correction remain in the independent review
history. They are distinct from the correct 64 **WorkUnits** difference between two completed
samples. None of the failed checkpoints is relabelled as a pass.

## Source and Evidence

| Qualified Rust path | SHA-256 |
| --- | --- |
| `crates/baozi-format-gltf/src/parser/accessor.rs` | `cfadbdbc735f01184a49cc16d36cc25c6c37ffe752be76915415c7b4eb8a9658` |
| `crates/baozi-format-gltf/tests/base_directions.rs` | `9e33a4883bc915827f2a25e1aa18a4d9995fa31f23bc6ceeacc4c1f5f6fced0a` |
| `crates/baozi-format-gltf/tests/limits.rs` | `f26c459683966c5ae899a363cb164a43bee621a1b49ed5d4e5f80249242293f1` |

Local evidence under `target/review/convergence/`:

- `gltf-base-direction-qualification-1.json`, SHA-256
  `1f13ba2f293b9bf7346bee6758728e606041958dc16965b5b545254c5093ac00`,
  contains all five exact commands, original logs/hashes, exit codes and before/after source maps.
- `gltf-base-direction-source-commit-1.json`, SHA-256
  `95179867b4f00f94301be53c19e1568dc9be87788472b733f7e90cbc15154427`,
  verifies every qualified Rust blob against source commit `0484dcb`.
- `gltf-base-direction-red-1.{json,log}`, `green-1.log`, `all-gltf-1.log` and
  `focused-2.log` preserve the failed checkpoints above.

The independent policy and final source reviews are retained under
`.git/baozi-review/performance-convergence/gltf-base-direction-{policy-review,final-source-review}.md`.
The final source review accepts the three Rust paths and two supported-profile additions;
its documentation finding about an "unread" sample is corrected in the completed plan.
No new normal estimator, MikkTSpace compatibility, animated implicit-flat evaluator, registry
publication, native promotion or performance clearance follows from this slice.
