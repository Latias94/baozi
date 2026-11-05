# Scalar Semantic Execution and OBJ Lexical Binding

This production cut addresses two repeated costs identified by the
[parser/pipeline investigation](2026-11-05-parser-pipeline-root-causes.md):
general callback transactions around scalar semantic checks, and repeated
service acquisition/layout binding in OBJ lexical scanning. It changes the
responsible execution interfaces without weakening input or stored-output
validation. Performance qualification is pending; the locating diagnostics are
not measurements of this change.

The starting checkout is `ffd0ee6`, whose production source is `56eecd9`.
Operation/Core and OBJ implementations have disjoint write scopes. Cargo and
nextest execute serially with one build job and one test thread, sharing the
root target directory. Original user notes remain outside the change.

## Copy Units Own a Reusable Physical Frame

`CanonicalSemanticWorkSession::run_copy_unit` accepts a Copy callback and returns
a Copy fact. These bounds exclude owned callback/result destructors at this
boundary. They do not assert that arbitrary callbacks are pure: a callback may
refer to a Cell or panic, so the host still owns exact physical work and unwind
resolution.

The session retains one actual `WorkFrame<1>` between compatible scalar units.
The first unit uses the existing `admit_observed` kernel. Each later unit uses
the existing singleton `replenish_one`, admitting only the next real unit before
its physical action. Every normally returned action completes literal one,
including false predicates and returned domain errors. The same ordinary
action-debt owner controls cancellation/deadline checks.

The completed frame has zero remaining work but an open producer. It closes
before an actual due poll, explicit semantic poll, storage operation, general
owned callback, outer semantic-body capture disposal, or final completion.
On panic, the numeric guard resolves the actual incomplete suffix and the
semantic guard latches the activation contract fault. An internally caught panic
cannot turn the pass into a successful completion. A later cleanup fault is
recorded without replacing the preserved first cause.

The old `run_unit` contract remains necessary for callbacks or results that own
destructors. It is no longer the scalar-payload path. There is one numeric
admission/completion kernel, no guessed whole-loop charge, no detached success
counter and no second policy implementation. This cut removes per-scalar
`catch_unwind`, owned-callback disposal and producer open/finish operations where
the Copy contract makes them unnecessary. It does not move work outside import.

Core's private authoring control surface forwards Copy units to the new session
method. F32 and F64 finite checks return a boolean through this path, then form
the checked nonfinite scalar error after the existing work/counter/periodic-poll
sequence. Integer payload inspection uses the same unit path. F64 remains F64;
the first invalid scalar and its offset remain checked. Actual retained-data
validation and the format's semantic checks are unchanged.

## OBJ Binds Lexical Work Once per Invocation

The parser's invariant `TokenWorkLayouts<'op>` binds DecodeWork and
Tokens/TokenBytes once in the real parser region. OBJ lines borrow those
layouts. The separate material-library invocation binds its own layouts, shared
across the MTL files it loads. A layout cannot escape or be substituted into
another invocation.

The character inspector receives an offset and character, without a service
capability. Successful character inspection no longer creates `ParseServices`
and repeats terminal/reentry preflight. OBJ reports NUL as a format-error
producer; only that cold branch obtains services to materialize the error.
Checked line-column overflow still uses the existing error view. MTL's no-op
inspector acquires no service.

Unicode character decoding, fixed initialized scan scratch, chunk boundaries,
comments, whitespace, token order and recovery authorization remain in the same
positions. Every scan chunk and token retains its actual admission, completion,
storage growth and control checkpoints. Per-line token storage is not pooled or
reclassified. The old service-bearing inspector and per-line layout-binding
paths are removed.

This deliberately leaves the other measured OBJ costs visible: per-token child
frames, line container lifetime, the two whole-text conformance passes, geometry
remapping and common scene construction. It is not a complete OBJ-parser rewrite
or a claim that its 44.78% record interval has been eliminated.

## Review and Verification

The independent bounded design review identifies a required outer-body cleanup
frontier: close the reused frame before destroying the outer semantic callback,
including the outer panic path. The implementation includes that correction.
Cleanup returns the preserved first fault, and a failed numeric resolution
retains fail-closed ledger/header evidence rather than admitting a fresh frame.

Existing regressions now cover both general and Copy semantic units at exact,
one-short and zero capacity, 255/256/257 cancellation frontiers, returned errors
and internally caught panics. Outer callback capture/drop/panic tests start from
completed Copy units. One mixed-boundary test crosses Copy execution, scratch
reserve/resize/validation, an owned callback and an explicit poll. A compile-fail
example rejects an owned String capture on the Copy API. The general destructor
tests remain relevant to the still-supported general callback contract.

OBJ adds one cross-library regression: a later Unicode MTL record's token-byte
limit must win before an earlier invalid numeric token is interpreted. It checks
the exact DecodeWork/Tokens/Records/Vertices/Faces prefix and complete live
storage release. Existing NUL, Unicode/recovery, 4-KiB scan, token-limit and real
completed-face cancellation tests remain part of package qualification.

The initial focused Operation/Core run passes 94 tests. The corrected production
source then passes seven serial gates: 3,590 workspace runtime tests with six
existing skips, 188 documentation tests, 138 external asset-consumer tests with
one existing skip, both strict all-target/all-feature Clippy checks and both
format checks. All 908 recorded source inputs are stable across that run.

Independent production review finds no blocking defect in the twelve changed
Rust paths. Test review requests explicit numeric assertions for a caught Copy
leaf panic and an independent consumed-limit denial. Both are added to the
existing private harness. They assert real attempted/consumed/pending counters,
no denied callback effect, no reuse after the caught panic and preserved finish
errors. The follow-up passes 25 focused tests, final workspace strict Clippy and
format checks. Its source map differs from the seven-gate source in exactly one
test file; all production, consumer, documentation and other test inputs remain
unchanged. This is not a claimed second complete workspace runtime run.

The final bounded test review closes both findings. Retained receipts are
`target/review/convergence/semantic-lexical-qa-{1,2}.json`; private production and
test receipts use the matching `semantic-lexical-*review*` names under
`.git/baozi-review/performance-convergence/`. Their final identities are:

| Receipt | SHA-256 |
| --- | --- |
| Seven-gate QA | `35faf7d569c6d03a3ba25e600a2ffe8d2e3b9e2630c1bc762b12cff00360a34e` |
| Test follow-up QA | `a1502acd4ea442aa1664ff4d0637c0f38cc4625e9523d5c7c014a6982c8f8c83` |
| Production review | `24d5832f589bb022b8addcd83e42ae8a39b32942f25036cc9c19a9ef3d5a518f` |
| Final T1/T2 review | `e13f15fe92dde78494e6e43b5ca85e9b60e63ed3efa77edb357e10f499023130` |

No native timing, allocation or RSS improvement is claimed by compilation or
correctness tests. Performance convergence remains unqualified for this source.

## Remaining Convergence Work

Measure the qualified source against the unchanged ordinary worker/corpus and
retain every original lane, failed sample, load record and regression. The
14-case/110-lane Windows/Linux release criteria remain unchanged.

Subsequent cuts still need to address actual repeated text-prefix execution,
token/container lifetime, glTF normalization, fixed import/preparation costs,
terminal ownership and any demonstrated staging/validation duplication. Treat
the distinct input and retained-output trust boundaries explicitly. Removing an
unnecessary internal check is valid when the new ownership/interface proves its
invariant; calling an earlier check proof of different stored bytes is not.

The next concrete removal is integer attribute value inspection: semantic
preflight runs empty callbacks per U32/I32 value, while retained validation visits
those values with an always-true predicate. Remove those loops and their work
quantities together. Integer storage, bytes, real copying, sparse indices,
topology indices, polygon counts, schema and range checks remain. Import, existing
storage, selection output and space-conversion output each own a validation-work
formula; all four must exclude the actual integer attribute values/defaults.
Selection must use output data lengths, which may grow during triangulation.
Preserve exact envelope finish rather than padding it with nonexistent work.
