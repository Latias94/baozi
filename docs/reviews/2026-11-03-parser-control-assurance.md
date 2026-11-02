# Parser Control Assurance

Status: the bounded parser-control slice is qualified at implementation
`98abe39680cc339b92460c46c21de98d51a5b4eb`. This receipt is not a release or final
performance result. It continues P6 of the
[operation architecture plan](../plans/2026-11-02-1700-perf-import-operation-architecture-plan.md)
after the typed-context implementation `89f4e0bba832a43be55ce60159cc3b1790040820`.

## Findings and Corrective Contract

New tests set an ordinary cancellation token immediately after an actual completed physical
parser action. Compile-gated local markers only observe the completed vector/record/index
and raise the token. They do not poll, complete work, allocate per item or alter the release
path. Baseline geometry, exact completed ledger prefixes, zero publication, cleanup, retained
old output and fresh-control plan reuse are checked before the responsiveness assertion.

The first executed results retain the gaps rather than changing the test bound:

| Actual path | Token raised after | Further fixed actions before stopping | Result |
| --- | ---: | ---: | --- |
| JSON glTF morph VEC3 | 257 rows | 3,839 rows | Violates the 256-action bound |
| GLB morph VEC3 | 257 rows | 3,839 rows | Violates the 256-action bound |
| Binary STL color expansion | 1 row | 4,095 rows | Violates the 256-action bound |
| Binary LE PLY wide list | 1 index | 4,095 indices | Violates the 256-action bound |
| Binary BE PLY wide list | 1 index | 4,095 indices | Violates the 256-action bound |
| OBJ completed face | 257 faces | 0 further faces | Passed |
| ASCII/binary STL completed facet | 257 facets | 0 further facets | Passed |
| ASCII/LE/BE PLY completed face row | 257 rows | Bounded by existing row checks | Passed |

Runs: `edb9ef01-e038-4509-b2ad-c8757483e3f5` and
`cdff9d56-6ee1-43da-ab14-ebcb2719a70e`. The unchanged legacy STL color test passed separately
at `2e1aa790-4d68-4cf4-bcdc-f96b8309946f`, demonstrating why a historical 4,096-row expectation
could pass while the accepted geometry contract was violated. These are actual failed
qualification observations, not timing measurements or final passing evidence.

The production correction separates the existing 4,096-byte text frontier from the single
shared `MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS` policy. Numeric collectors must count actual
bounded scalar/vector/index/matrix/comparison leaves, including steps inside a wide row.
The constant does not turn an input-sized callback into a fixed action, and does not promise
elapsed-time interruption of IO, allocation, extensions or other opaque calls.

Inspection also found OBJ comparison chunks that completed their full admitted tail at
normal finish. That can label unused comparison credit as consumed and delay physical
completion. Its correction must retain admission before the comparison and record each
actual completed leaf immediately, with control cadence independent of that completion.
Eight focused comparison/chunk tests and independent source review qualify this separately
from responsiveness. Numeric, byte and length comparison admission precedes the physical
leaf; actual completion follows immediately. A failing later source push preserves the exact
earlier completed prefix and releases the unused admitted suffix.

Independent glTF review found four additional gaps: non-ASCII JSON key hashing could cross a
byte frontier before its control check, adjacent semantic-duplicate scans were unbounded,
required-extension membership compared input-sized names without byte checks, and resource
source-identity construction lacked row checks. All four are corrected. The new probes use
wide attribute tables, long common-prefix names, actual external resources and actual JSON
key digest boundaries. The JSON crossing negative control failed at physical byte 4,099 while
only 4,095 bytes were reported complete. That first assertion prevented its GLB branch from
executing; the red observation therefore establishes JSON only. The final green tests execute
both JSON and GLB, including exact-fill, crossing and unwind cases.

## Public Behavior and Progress

The public assurance matrix uses actual OBJ, ASCII/binary STL, ASCII/LE/BE PLY, external-buffer
JSON glTF and GLB inputs. It covers pre-cancelled entry, primary-open return, expired deadline,
validation observer cancellation, final signal/observer cancellation, successful detached
geometry, plan reuse, unknown/hint primary totals and sidecar-bound cancellation. It does not
label the Validate event as an interruption inside parsing. Local format tests supply that
separate evidence.

Current progress delivery covers acquisition, detection, validation and final publication.
Decode/Normalize vocabulary does not imply fine-grained events inside every first-party
parser. Phase/unit completed values and sequence numbers are monotone, unknown totals remain
unknown, and zero transient delivery bounds do not suppress the mandatory final event or
cancellation check. No percentage is invented for phases without a total. The controlled
example in `crates/baozi/examples/import_control.rs` shows the public token/deadline/observer API.

One initial facade assertion incorrectly equated aggregate WorkUnits with opaque-open work.
The corrected oracle compares bound-query cancellation with open-return cancellation for
the same fixture, isolating exactly the backend's declared open amount from startup and
protected terminal work. It preserves zero reader calls and zero payload admission.

## Qualification

| Check | Observed result |
| --- | --- |
| Shared text unit tests, including six physical-frontier cases | 5/5; `e9bea1ed-9637-4d32-ae22-98d04187c78a` |
| Operation execution/control tests | 16/16; `afabc0ec-a2d0-4990-8d69-7fcb9f7c413d` |
| STL/PLY physical parser controls | 9/9; `dac5738d-de84-4925-861a-b1d3fcaea9ba` |
| Complete STL/PLY semantic/resource tests | 166/166; `2999f01b-d4e8-4e18-97b3-0b08791be13d` |
| Complete OBJ tests, including eight comparison/chunk regressions | 164/164; `c5b7e622-78bf-416d-9d51-2841e82478d1` |
| Fresh glTF physical controls and four review corrections | 9/9; `eeb7b914-8bab-4c52-b50a-564c29d58e6c` |
| Final complete glTF semantic/resource tests | 508/508; `3b961666-e6c2-41f8-bab2-f07512bf7ee3` |
| Final public facade matrix, all eight encodings | 9/9; `3a8b69f7-6afb-403b-afdd-da544c6a8fad` |
| Public facade matrix with only OBJ enabled | 9/9; `6d48633a-d030-4ef7-bedf-641b8738318b` |
| Controlled import example | Executed successfully; actual acquisition/detection/validation/final events retained |
| Final workspace all-target/all-feature strict Clippy | Passed with `-D warnings` |
| Final workspace all-feature doctests | 63/63 passed, including lifetime and compile-failure contracts |
| Final workspace format check | Passed |
| Independent source reviews | Public facade, shared text/control, STL/PLY, OBJ and glTF closed; no open actionable finding |

The shared text test triggers after a real classified byte in each of the planning/build
passes. It checks ASCII, a four-byte UTF-8 unit starting at offset 4,095 and a later invalid
byte permitted by replacement mode. Each selected pass stops at its exact frontier, without
splitting the UTF-8 unit or classifying the later invalid suffix. DecodeWork includes the
complete planning pass when cancellation occurs during building; no completion replay is used.
The initial test-profile omission of required RejectInvalid support is retained as a failed
fixture setup, then corrected without changing production text policy. The OBJ test oracle
initially compared control failures to a format-failure baseline, which adds two WorkUnits for
failure construction. Its replacement uses actual immediate token cancellation as the matching
baseline; physical completion and denied-admission assertions remain exact. Test-only Clippy
corrections introduce no lint allowance or production-policy relaxation.

## Source and Evidence

`target/review/p6-tested-source.json` inventories all 49 implementation paths and their tested
SHA-256 values. Each matched before staging. The source commit contains the four parser fixes,
shared policy, regression tests, public example, README and current ADR amendments. The draft
receipt and unrelated user notes were excluded from that commit.

The JSON negative control initially shared generated Cargo artifacts with the main source.
The first purported green run and the timestamp-only retry reused its failing executable;
neither is accepted as main-source qualification. Cleaning only generated
`baozi-format-gltf` package artifacts and rebuilding produced the fresh 9/9 run above, followed
by the complete 508/508 run. `target/review/p6-json-cache-boundary.json` retains this boundary.
Future source variants need distinct generated artifacts for reliable negative controls.

The evidence inventory and archive retain successful and failed logs, exact commands, source
inventory, the committed source archive, independent reports and red-source facts. Historical
failed structured receipts are not overwritten or relabelled as green. Their successful retry
logs and the final `p6-final/verification.json` supply the passing results.

| Artifact | SHA-256 |
| --- | --- |
| `target/review/p6-baozi-source.tar` | `12388bf5f5072e8508d6c540aaca7a0a51641911762b5d04c178080dc682dd14` |
| `target/review/p6-baozi-control-evidence.json` | `0ea317611131284479cbd85a6e226fc266d13637123364be5511c022d6ff07b4` |
| `target/review/p6-baozi-control-evidence.zip` (75 files) | `3bdba72bf998422a17ca776e5c225f7bab3186795790322dc44dddb73003ef8d` |

## Remaining Work and Limits

The scoped implementation and control tests are complete. Assimp's separate
[two-host observations](2026-11-02-assimp-control-observations.md) do not prove Baozi's matrix.
These probes do not constitute a proof over every parser loop or elapsed-time responsiveness.
Standard numeric conversion, allocation, IO and external callbacks remain cooperative opaque
boundaries. Progress is phase-oriented; uniform internal Decode/Normalize events remain a
separate product improvement. The complete feature matrix and final host/portable refresh
belong to original U14. P5 attribution, final-source Windows/B1/heap observations and original
U14 remain open, and the overall goal remains active.
