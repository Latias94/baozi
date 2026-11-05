# Work Layout Arity: Source Qualification and Instruction Diagnostics

The `b09f4c1` arity specialization is retired at `c497c90` after full GNU ordinary sampling gives mixed results and Windows candidate sampling cannot start. Production returns exactly to the qualified e5 work kernel. Six instruction diagnostics decrease by 1.229%–4.147%, and large repeated OBJ pipeline observations improve, but this evidence does not establish a defensible cross-platform performance keep or release convergence. Every favorable and unfavorable result remains retained.

## Source and contract

Candidate `b09f4c145003da2e44e70a63c67f1c80f85a1ae9` changes only `crates/baozi-operation/src/ledger/work/kernel.rs`, against runtime control `e5ca93b82f2041e82e6bbaa6913462be551bd69f`. Both empty and singleton layouts have identity representative and completion mappings. Singleton aggregation is the input amount: there is no alias and adding it to zero cannot overflow. The catalog row may have any valid position; layout slot zero is not catalog row zero.

Private inline accessors expose these immutable facts to monomorphization. Existing fields and the general N>=2 alias path remain. Actual arithmetic, policy, channel/header validation, admission, completed physical prefixes, limits, cancellation, unwind and physical work quantities are unchanged. This is one checked kernel, with no new public API, numeric policy, unsafe code, dependency or detached counter.

Independent design/source review checks construction sites, lack of later map mutation, the valid test constructor and existing alias/nonzero-position/overflow/denial/prefix behavior. Focused Operation nextest 1,074, doctests 128, strict all-target/all-feature Clippy and formatting pass on 908 stable source inputs. The seven integrated gates also pass: workspace nextest 3,593 with 6 existing skips; doctests 188; independent asset consumer 138 with 1 existing skip; strict all-target/all-feature Clippy and formatting for both workspace and consumer. All 908 Rust/TOML source inputs match before and after the complete run. The actual integration process exited 0; ordinary performance acceptance remains separate.

## Separate instruction observations

GNU symbol workers use Rust 1.95, opt3/debug1, generic x86-64, System allocation and no allocation-metrics. Each version/task has one observation. Candidate source is an immutable commit archive, unchanged before/after build; the controls are historical same-policy e5 profiles. Whole-worker Ir includes setup, preload, checking, serialization and teardown.

| Original task | e5 Ir | Candidate Ir | Change |
| --- | ---: | ---: | ---: |
| glTF shared 1 | 5,967,110 | 5,893,791 | -1.229% |
| glTF shared 32 | 29,982,679 | 29,405,794 | -1.924% |
| F64 PLY neighbors | 2,512,824 | 2,481,108 | -1.262% |
| STL repeated 1024 | 23,642,326 | 22,952,854 | -2.916% |
| OBJ repeated 16384 | 810,329,922 | 776,727,337 | -4.147% |
| OBJ grid 128 | 2,761,416,772 | 2,702,590,782 | -2.130% |

Independent review reconciles twelve raw profiles, their summary/totals/Collected, twenty-four exclusive annotations, archive/build/binary identities and aggregate outputs. Both grid logs retain `brk segment overflow`; this instrumented warning remains visible rather than qualifying native execution.

The collector used obsolete optional `import_work_units` / `work_budget_applied` keys. Those comparisons were skipped. Independent direct checks of the actual required `import_work` / `import_budget_enforced` fields reconcile all six original output pairs. Main repeated those required-field checks after collection. The original script is retained verbatim; the corrected script requires actual schema keys, and the summary preserves and corrects its original attribution. The original runner did not establish work equality; independent raw review and recorded post-collection checks do. No raw result was overwritten or recollected to repair this finding.


## Ordinary sequence outcomes

Both versions pass the original 110 functional lanes on both platforms, with zero warmups and one retained pair per preflight. These results do not qualify timing. Ordinary workers are opt3/debug0, generic x86-64, System allocation, without allocation-metrics; both candidate builds use the same immutable archive as the diagnostic worker and retain 1,472 unchanged source files.

| Platform/sequence | Outcome | Sampled named-tool qualification |
| --- | --- | --- |
| GNU C0 | 110 lanes / 5,062 commands, all successful | Linux 0/53 nonempty; Windows 13/52 nonempty |
| GNU N0 | 110 lanes / 5,062 commands, all successful | Linux 0/54; Windows 0/53 |
| GNU N1 | 110 lanes / 5,062 commands, all successful | Linux 0/56; Windows 0/54 |
| GNU C1 | 110 lanes / 5,062 commands, all successful | Linux 0/55; Windows 9/53 |
| Windows C0 | 110 lanes / 5,062 commands, all successful | 124 nonempty tool snapshots |
| Windows N0 | No samples: no 20-second named-tool quiet period within the fixed 180-second start wait | 90 nonempty wait snapshots; N1/C1 do not run |

The Windows parent wrapper reports terminal status 1; its explicit stop receipt records the start-limit frontier. No Windows candidate timing, memory change or source effect follows. The first GNU wrapper also terminates without preparing a task: an incorrectly retained underscore-named entry points to the previous e5 runner, whose existing-directory refusal stops it immediately. That wiring failure, original scripts and logs remain. A corrected prospective entry and fresh window2 workspace then produce the four complete GNU campaigns; no previous sample is overwritten.

All native tasks, 14 cases, 110 lanes, 3 warmups and 20 retained AB/BA pairs are unchanged. Per-pair B/A medians invert each retained A/B value individually before taking statistics; separate version campaigns are not fictitiously paired. [All Linux lanes](2026-11-05-work-layout-arity-linux-lanes.md) retain every positive and negative stage change, absolute medians/IQRs, candidate maxima, comparator ratios and whole-child RSS ratios.

| GNU campaign | Timing lanes above 3x | At or above 10x | Pipelines above 3x | Maximum median whole-child RSS B/A |
| --- | ---: | ---: | ---: | ---: |
| C0 | 69/110 | 32/110 | 14/28 | 2.048 |
| N0 | 70/110 | 32/110 | 14/28 | 2.043 |
| N1 | 69/110 | 31/110 | 14/28 | 2.053 |
| C1 | 69/110 | 31/110 | 14/28 | 2.045 |

These are observed threshold counts, not release acceptance. Control load overlaps limit causal version attribution; absence of sampled tools is not global host quiet. Whole-child RSS is separate from endpoint heap and Windows commit. No fresh candidate heap attribution is collected for this arity-only change; the prior e5 heap result is not relabeled as b09.

Repeated OBJ memory pipeline falls from 46.304 to 42.435 ms in C0/N0, and from 46.510 to 43.225 ms in C1/N1, while Assimp medians stay around 9.17–9.31 ms. Grid memory pipeline falls initially but increases in confirmation. STL and glTF32 memory pipeline Baozi medians increase in both orders, although their same-campaign comparator ratios generally improve. These observations disagree with a uniform-gain interpretation.

Small memory extraction also repeatedly increases. PLY triangle changes from 5.336 to 7.021 us and 4.409 to 7.020 us; shared glTF1 from 6.655 to 8.016 us and 6.377 to 8.685 us; F64 PLY from 5.426 to 6.791 us and 4.957 to 6.638 us. Keep absolute scales, IQRs, control drift and load qualifications alongside relative percentages. Large grid memory extraction decreases in both comparisons. Independent disposition review retains these tradeoffs and recommends rejecting the specialization on present evidence; it does not establish that source alone caused every change or that a workload-weighted net benefit is negative.

Each complete GNU raw archive is approximately 41.7 MB. Standard tar plus copy takes 1.08–1.60s after collection, versus the prior 386.453s per-file copy. Native raw originals, complete archives and standalone reports are retained. This operational improvement is outside benchmark clocks and is not a Rust speedup.

## Next cost owner

The large source-finite call chain does not isolate guard cost. A proposed closed F32 span would touch core control, semantic session, execution and a prefix/unwind bridge, and would introduce numeric policy into operation. It is deferred pending narrower existing-owner evidence. A whole-slice precharge or success-only batch completion has denial/panic/cancellation counterexamples. Retain actual per-action checks unless a qualified owner redesign establishes an equivalent boundary; avoid adding a generic proof/ledger framework for this task.

The prior Linux nativefs sequence spent 105.757s collecting one full control matrix and 386.453s copying its 15,193 artifact files to Windows. Future original-matrix collection uses standard tar after collector terminal and copies a single archive plus standalone report/receipt. All raw native originals remain. Archive/copy time remains outside child stage and parent elapsed clocks; all original semantic tasks, 110 lanes, 3 warmups/20 retained pairs and load gates are unchanged. Earlier stopped attempts remain separate. Retrospective correlation of that control collector finds 53 Linux and 52 Windows named-tool snapshots with no observed tools; this is sampled scope, not global quiet. The following N0 start waits contain 91 Windows snapshots with tools, and no candidate ordinary sample exists for that attempt.

## Local evidence

- `.git/baozi-review/performance-convergence/work-layout-arity-experiment-plan-1.md`, `work-layout-arity-design-review-1.md`, `work-layout-arity-instruction-review-1.md`.
- `target/review/convergence/work-layout-arity-focused-1.json`, `work-layout-arity-integration-qa-1.json`.
- `target/assimp-comparison/convergence-linux-work-layout-arity-symbols/{source.tar,build.json,build.log,baozi-probe}`.
- `target/review/convergence/work-layout-arity-attribution/candidate-*`, `work-layout-arity-attribution-main-summary-1.json`, `work-layout-arity-work-field-correction-1.json`.
- `.git/baozi-review/performance-convergence/scalar-validation-batch-investigation-1.md`; `target/review/convergence/work-layout-arity-copy-owner-source-rows-1.json` contains only textual exclusive source-row attribution, not isolated guard costs or expected savings.

## Source disposition

Main accepts the independent recommendation to retire this bounded experiment. An established 3.3–3.9 ms OBJ pipeline benefit would matter more than isolated 1–3 us extraction changes for an OBJ-heavy workload; unequal control load prevents claiming that established source benefit here. Other pipeline costs and absent Windows candidate timing remain open. This is not a correctness rejection or a claim that all arity specialization is inherently slower.

Before reversing the one-file experiment, main checks that the kernel still has the exact committed b09 bytes and no user edits, with an empty staging area. The exact reverse patch is applied only to that path and committed as `c497c90`. All 908 Rust/TOML inputs then match the previously fully qualified e5 receipt byte-for-byte. No new Cargo run is claimed for this identical restored source; prior integrated and ordinary-control qualification is reused. The five original user-note hashes remain unchanged. Archive/binary/raw evidence is not rewritten.

Independent disposition receipt: `.git/baozi-review/performance-convergence/work-layout-arity-native-disposition-review-1.md`. Independent raw review reconciles all 25,300 original ordinary worker stdout observations across the five completed campaigns, their stderr, 20,240 GNU time RSS records, 5,060 Windows serialized process-memory results, all 550 lane summaries and 220 version-change rows. It also checks all four complete raw tar archives and immutable worker/source associations without a material discrepancy. Windows memory verification checks recorded process-API results and arithmetic, not a second measurement. Its receipt is `.git/baozi-review/performance-convergence/work-layout-arity-native-raw-review-1.md`. The experiment's production net change relative to the pre-experiment main is zero. The operational archive improvement applies only after collection and is retained in local experiment orchestration.

The next investigation targets repeated source-text conformance and per-character parser execution, plus accurate extraction-boundary attribution. It must distinguish already-proven immutable source facts from newly constructed line/recovery/output layout and live work/control state. Avoid another cross-layer proof/prefix framework without a measured owner opportunity. All Windows/Linux endpoint, pipeline and comparable whole-child memory release gates remain open.
