# Closed Scene Edit Publication

This continues the [lexical invocation-host plan](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md)
after the [typed recovery checkpoint](2026-11-04-canonical-recovery-host.md). The actual Rust
successor remains in the isolated compiler workspace. Main Rust, numerical algorithms, comparison
tasks and the unchanged Windows/Linux 14-case/110-lane release gates are unaffected. There is no
new ordinary performance result.

## Concrete Behavior and Removed Interface

Previously, core's Scene-edit generation consumer accepted caller-owned rejection/preparation
callbacks and an arbitrary result type R. A successful handoff then decoded and checked evidence
after transferring its prepared store. An unexpected mismatch could therefore reject after the
irreversible terminal transition.

The public consumer now returns one fixed owned `SceneEditPublication`: success, rich failure or
minimal fallback. Its only errors are precommit containment and preparation. The public token,
joint, frozen-generation-rejection and pairing-error exports are removed. Postprocess matches
the actual owned result through its existing closed cause/resource projection; it supplies no
terminal callbacks. There is no compatibility wrapper retaining the old public callback/R path.

Central validation carries reference-free handoff facts in the actual `SceneEditValidationProof`.
Facts derive from the same recipe words, digest, units and exact remap references already checked
by metered visitors. Canonical correspondence and graph validation still finish before accepting
the proof. Existing operation-kind, source-target-generation, normal/tangent and space predicates
move before acceptance. No source Scene, borrowed recipe, scratch owner, alternate decoder,
unmetered evidence rescan or allocation is stored in the proof.

Before reserving a destination generation, the concrete driver admits the same-root vacant
recovery slot and both actual canonical/evidence roots. A denied owned source is disposed while
output/proof/storage remain live, then the existing owner is contained. Matching admission
returns the original permission and source. Success handoff is private and infallible: it moves
the proven tables, evidence and generation into the owned result without decoding or calling
caller code. Existing late joint-failure cleanup remains a separate outer-driver obligation.

`OutcomeEvidenceValidationMember::reference()` copies only non-owning identity. It can be saved,
compared and used with an already matching published owner; it supplies no storage ownership,
live lookup authority or validation-success witness. Data inspection during validation still
uses metered visitors. The new borrowed proof getter retains the host's actual proof owner.

The fixed result keeps its bounded Scene owner inline. Its explicit large-variant lint rationale
avoids introducing an allocation after commit. No provider trait, new ledger/coordinator, unsafe
operation, stronger error lifetime bound or normal-math change is introduced.

## Frozen Source and Actual Windows Checks

Eight files differ from qualified typed recovery source4: three operation files, four core files
and the actual postprocess caller. The complete snapshot contains all 1,405 inputs.

| Artifact | SHA-256 |
| --- | --- |
| Complete source set | `f5483ee14ba8b45befd368b8006053b338b1dbfc5b472e0876828f944a161111` |
| Source manifest | `f618b68263d99572fa626a28ae955b86f5a67bdc637e8d1da107ce5c06c4516b` |
| Integrated affected/consumer QA receipt | `ac370607245beba7a1a19fe6a2990c305b80e67292c001bd869328fe5be5db9f` |
| Separately completed operation QA receipt | `e5c342c091dd2822ac593ab6fc1873fb07419261358713e1a537e76bbd817a83` |
| Independent operation source/evidence review | `2df932cedf84b311bf5f4279cd8e5796f308ba96a3baf63ebf2c22687ebe3249` |
| Independent core/caller source and final QA review | `b3c73009a0bc24dd066ded1e7428ba7c1e5059c437a72b4a1e562215f1ed4554` |

| Actual check | Result and scope |
| --- | --- |
| Operation pairing regressions | **3 passed**; run `d3913e65-c923-4b6b-8466-f1771d773303`. Other tests were filtered. |
| Complete operation nextest | **938 passed**, no skips; run `175c64ae-41e2-4b70-bfe3-9cdd8dc07e75`. |
| Operation doctests | **58 passed**, including existing negative compiler contracts. |
| Connected core/postprocess focused regressions | **9 passed**; run `8ba04539-4899-4a42-8654-8cae415872e1`. Other tests were filtered. |
| Complete core/postprocess nextest | **986 passed**, 6 existing skips; run `557db2de-9b9c-4040-86f5-82caefecd527`. |
| Core/postprocess doctests | **10 passed**, no failures or ignores. |
| Independent asset consumer nextest | **138 passed**, 1 existing manual skip; run `137b6be4-c7d9-4a1d-aac3-9ec8363fa7d6`. |
| Workspace and consumer strict Clippy | PASS, all targets/features, `-D warnings`. |
| Workspace and consumer format checks | PASS. |

All eight integrated QA commands return exit zero. Every source hash matches the manifest before
and after the sequence, and every recorded log digest matches its actual bytes. The separately
checked 116 operation/common inputs also match this integrated source. The earlier operation
package result therefore remains qualified on its unchanged inputs; it is not a new complete
workspace-runtime claim. The predecessor's 3,436-test result belongs to its own frozen source.

The actual tests exercise each foreign root independently and together, source-capture Drop
panic before containment, matching-root success with real commitments, and the non-unit core
proof through actual space conversion, generation exhaustion and a substituted canonical proof
root. Selected remap and changed/failed revision tests retain their existing data and cleanup
assertions. The private handoff's function-type assertion fixes its return as infallible.

Independent operation review finds no actionable defect in its three-file scope. It names
coverage limits for direct custom output/proof destructor ordering and identity export followed
by validation rejection or foreign-owner lookup. Source tracing and the connected non-unit core
probes supplement that bounded review; no generic guarantee follows from an identity token.
Independent C/P review reports no introduced actionable finding and independently verifies the
full snapshot, predecessor chain, all eight actual command exits, complete before/after maps and
log hashes. It traces the old predicates, field offsets, signed/bit comparisons, source/output/
proof/evidence association and P first-cause/resource projections. The closed publication
component is source-qualified within this receipt's scope.

The C fixture directly substitutes a canonical proof root; independent O tests cover either
foreign root, while real C success checks exact evidence-root association. The moved malformed
descriptor predicates were compared with the predecessor, but this cut does not add an exhaustive
mutation matrix or an injected late-joint-failure regression. Both reviewers keep these coverage
limits explicit rather than claiming whole-pipeline qualification.

## Remaining Connected Work

The whole-invocation retained FnMut body, callback/scratch cleanup boundary and reference-free
host pending stages are not implemented by this component. Public operation preparation/freeze/
joint authority, import's callback/R publication and postcommit pairing tail, and borrowed
pending owners in the import cell remain to migrate. Late joint-failure token cleanup must be
resolved in that connected driver rather than described as fixed here.

Borrowed numeric backing, one current-state attachment/start admission, shared-shell-before-state
promotion, retired-path deletion and complete ordinary Windows/Linux timing/memory qualification
remain open. None of the checks above establishes performance convergence or release readiness.
