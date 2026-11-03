# Compact Owner-bound Work Capabilities

R2 of the [performance convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md)
replaces copied semantic keys inside repeated numeric work capabilities. It follows the retained
[semantic execution experiment](2026-11-03-bound-semantic-execution.md). This receipt initially
qualifies source and correctness; ordinary measurement is a separate keep gate.

## Representation and Contract

`BoundWorkGroup` retains the actual local owner reference, positions, duplicate representatives,
completion order and grant. It no longer copies every full semantic key into each parser scope
and observed permit. Keys remain in the immutable owner catalogue. Six existing private numeric
helpers receive static lazy key resolvers and materialize keys only while constructing an error;
the two inline overflow projections are lazy as well. The cold parent-facing validation seam
still accepts its existing key and uses the same numeric helpers.

There is one numeric kernel and one counter/policy authority. This change adds no allocated
layout, hash table, narrowed unchecked index, callback seam, ledger backend or shadow counter.
Binding comparisons remain outside numeric borrows. Internal key resolvers only copy immutable
metadata; no user callback, policy query, formatting, allocation or control action runs there.
Each operation preserves current arithmetic/check/commit order, duplicate-lane error selection,
representable rejected attempts, atomic direct completion and observed physical-prefix failures.
Current policy, mode, protected-role and outstanding-permit checks remain live.

After a successful binding, numeric errors name the registered catalogue key. An absent-key
binding error still names the caller's supplied key. For a generic key whose equality ignores
diagnostic labels, this deliberately canonicalizes the bound error's label. Baozi's concrete
key compares the complete identity. The breaking refactor documents and directly tests that
choice instead of retaining a hot copied array for unspecified diagnostic representation.

## Source Review and Tests

The four-file candidate was prepared in isolated copies while the immutable R1 worker was
built/measured. Independent review projected all 26 patch hunks against the actual base and
matched them to those copies. Main applied the patch only after R1 collection and attribution
completed. The reviewer separately inspected the formatting delta and actual production hashes;
no actionable P1/P2 finding remains for that snapshot.

Four focused tests check zero resolver evaluation on successful helper routes, exact identity
and one evaluation on failing helpers, noncontiguous/duplicate protected positions, and the
registered/caller diagnostic-label rule. Parser tests exercise post-binding protection and
replenishment at a nonzero catalogue position. Existing duplicate/overflow precedence, physical
prefix, policy tightening, role revocation, zero/full forgotten permit, comparator panic and
no-Ord/no-Debug numeric tests remain intact. No layout snapshot or test mirroring field order
was added.

| Check | Result |
| --- | --- |
| Operation/core/import/postprocess nextest | 2,266/2,266; six existing skips; `df51a838-91b3-4a52-991f-f5f9dbfef41e` |
| Affected compiler contracts | Import 17; operation one positive and 39 compile-fail; passed |
| Strict affected all-target/all-feature Clippy | Passed with `-D warnings` |
| Package formatting | Passed |
| Real-format facade control assurance | 9/9; `23fd4876-c525-4535-aedb-57b45231d3e3` |
| Asset consumer | 120/120; one existing skip; `18e37e89-5072-4099-b7c9-74f7926ed4fd` |
| Ordinary performance | Pending; this source qualification does not decide the experiment keep |

Commands use Rust 1.95.0, locked/offline inputs, one build job and the shared target. Exact source
hashes and command logs are `target/review/convergence/r2-*.json` and corresponding logs.
The patch, implementation note and independent source review are in
`.git/baozi-review/performance-convergence/`. R1 instruction reductions are not credited to R2.
Source-level laziness and a smaller private capability do not establish native speed, smaller
return ABI traffic, memory superiority or Assimp convergence; ordinary measurements decide that.
