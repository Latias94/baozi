# Text Emission After Source Conformance

Candidate `7900255` separates build-time encoding navigation from source policy qualification. The successful private planner already accepts the same immutable source bytes and copied facts/options immediately before building. Build now emits ordinary units, newlines and replacement spans without rechecking profile/control/lexical policy. The original planner and detector remain the acceptance boundary. Ordinary performance is mixed. Independent disposition recommends retaining this narrow source cleanup while closing its performance hypothesis as unproven; main accepts that distinction after completed raw review. No release convergence is established.

## Scope and unchanged obligations

The source change is confined to `crates/baozi-import/src/text_input.rs`. Private `TextEmission` shares existing newline/UTF-8 width helpers, replaces build's repeated classifier call and retires the planning-proven violation branch. It is not a new public validation bypass, source iterator framework, unsafe conversion, detached work counter or single-pass text parser. No source lifetime or storage API changes.

Actual emitted output still needs independent capacity, exact length, row/range and recovery checks. The counted prepass, BOM recheck and work, per-unit guarded read/completion, WorkCursor control frontiers, bounded copy, safe `from_utf8` conversion, raw/decoded replacement spans, line-index construction and transactional record cursor all remain. Source conformance cannot establish newly emitted layout. PLY boundary discovery still stops before binary payload.

Independent source review checks every builder call: one production call immediately follows successful planning with identical immutable locals, and one test-only call changes decoded capacity to trigger real copy failure. Invalid/overlong/truncated UTF-8 retains identical raw-unit widths; valid non-ASCII under AsciiOnly is rejected by planning, and the forbidden AsciiOnly/replacement configuration remains rejected during profile construction. No reachable new policy acceptance or work-prefix counterexample is found. Existing tests cover the meaningful boundary; no private enum representation test is added.

## Source qualification

The focused run passes 42 text contracts/private tests, strict Import all-target/all-feature Clippy and workspace format, with 908 stable Rust/TOML inputs. The subsequent affected run passes Import/OBJ/PLY/STL 818 tests with no skips, 49 doctests, 138 external asset consumer tests with one existing skip, strict affected/consumer Clippy and both formats. Its 908 input hashes remain stable. These are affected source/caller qualifications, not a newly executed whole-workspace release gate.

The candidate is committed exactly as `7900255d940dfc40952b8a0a0909396f79860740`; immutable archive contains 1,474 source inputs. Diagnostic and future ordinary builds use that archive. No native gain or main merge is inferred from the source tests.

## Separate instruction observations

GNU workers use Rust 1.95, generic x86-64, opt3/debug1, System allocation and no allocation-metrics. One whole-worker observation per source/task compares exact candidate to historical e5 controls. Setup, preload, checking, serialization and teardown are included, so these are not isolated stage or native measurements.

| Original task | e5 Ir | Candidate Ir | Change |
| --- | ---: | ---: | ---: |
| glTF shared 1 | 5,967,110 | 5,962,884 | -0.07082% |
| glTF shared 32 | 29,982,679 | 29,971,135 | -0.03850% |
| F64 PLY neighbors | 2,512,824 | 2,508,949 | -0.15421% |
| STL repeated 1024 | 23,642,326 | 23,640,064 | -0.00957% |
| OBJ repeated 16384 | 810,329,922 | 808,868,720 | -0.18032% |
| OBJ grid 128 | 2,761,416,772 | 2,747,170,762 | -0.51589% |

Independent evidence review reconciles twelve raw profiles, all twenty-four existing annotations, summary/totals/Collected, immutable build/binary association and actual required work/budget/output fields. All complete observations differ only by instrumented stage time. Both grid `brk segment overflow` warnings remain visible. Saved call trees show the build classifier replaced by TextEmission with unchanged observed unit counts; planning/detection classifier calls remain. No inclusive costs are summed into predicted savings.

These small reductions describe the observed scope of this cleanup. Encoding navigation, copying, per-unit guards and other work remain; these measurements do not quantify their share of the remaining cost or establish that cost as unavoidable. The observed builder call transition does not explain every glTF/STL whole-worker change. No allocation, whole-child RSS, Windows speed or release target is cleared. Do not expand this helper into a generic proof/prefix framework to justify its implementation.

## Ordinary outcomes

Both exact-archive candidate platform builds use opt3/debug0/generic x86-64/System without allocation-metrics, with 1,474 source inputs unchanged. Control builds retain their 1,470-input e5 maps. Both source versions pass every original functional lane on each platform. These preflights use zero warmups/one pair and do not qualify timing.

Windows completes fixed C0/N0/N1/C1, each 110 lanes/5,062 successful commands, with three warmups and twenty retained AB/BA pairs. Sampled named-tool overlaps are respectively 0, 0, 5 and 0. Absence of those samples does not prove global quiet. GNU completes C0/N0, each 110 lanes/5,062 successful commands; all GNU tool samples are empty, while Windows monitoring records 50 nonempty snapshots during C0 and zero during N0. GNU N1 does not start: 86 blocked observations fail to obtain a 20-second simultaneous named-tool quiet window within 180 seconds; C1 does not run. No confirmation is inferred.

| Platform/round | Timing lanes above 3x | At or above 10x | Pipelines above 3x | Maximum median process-memory B/A |
| --- | ---: | ---: | ---: | ---: |
| Windows C0 | 57/110 | 31/110 | 9/28 | 1.799 working set |
| Windows N0 | 57/110 | 32/110 | 9/28 | 1.799 working set |
| Windows N1 | 57/110 | 31/110 | 9/28 | 1.799 working set |
| Windows C1 | 57/110 | 31/110 | 9/28 | 1.798 working set |
| GNU C0 | 70/110 | 31/110 | 14/28 | 2.021 RSS |
| GNU N0 | 70/110 | 31/110 | 14/28 | 2.038 RSS |

These are descriptive same-campaign threshold counts, not release acceptance or isolated change effects. Maximum candidate median Windows peak commit ratios are 1.469 and 1.457; whole-child process metrics are not endpoint heap. No new candidate heap attribution is collected, and e5 heap evidence is not relabeled as candidate evidence.

[Every ordinary lane](2026-11-05-text-emission-lanes.md) retains absolute Baozi medians/IQRs, every version change, paired comparator ratios, retained candidate maxima and process-memory ratios. Main summaries invert each individual A/B sample before taking B/A statistics. Changes compare separate campaign medians, not paired e5/candidate versions; no samples are trimmed.

Windows memory grid pipeline medians decrease 1.713%/1.729%. Repeated OBJ pipeline increases 0.444%/1.587%; glTF32 changes +0.200%/-1.173%; STL1024 changes -2.198%/+1.302%. Small shared glTF1 memory extraction increases 9.13%/15.77%, and PLY point file extraction increases 11.50%/9.34%. Absolute scales, IQRs and tails remain in the companion and raw reports. Neither the small instruction reductions nor a favorable isolated pipeline establishes a uniform gain.

Independent disposition recommends source-cleanup retention, with no native benefit or convergence claim. Main accepts it after complete raw reconciliation. The repeated extraction costs and opposing pipeline/tail/elapsed observations remain repair/qualification obligations. Independent review reconciles all 30,360 original worker outputs, 10,120 GNU RSS records, 20,240 serialized Windows process-memory pairs, 660 lane summaries and 330 version-change rows. All four companion tables, 440 data rows and 2,310 numeric display cells match. Windows memory verification checks recorded API results and arithmetic, not a second measurement. A requested correction narrows the earlier unquantified remaining-cost statement above; original raw and summary files are unchanged.

## Next owner

The next deep candidate is actual typed scalar construction: assess a closed finite-payload owner, finite qualification fused with real writes, central validation of actual output layout/references and no mutable unqualified float seam. Full-source projected-value rules, edited/converted values, resource admission, exact physical work/control and owned failures remain necessary. Any revised unpublished error-discovery ordering must be explicit; first observed failure ownership must remain honest. This is design work, not an implemented invariant or predicted speedup.

## Local evidence

- `target/review/convergence/text-emission-focused-1.json`, `text-emission-affected-qa-1.json`.
- `.git/baozi-review/performance-convergence/text-emission-source-review-1.md`, `text-emission-instruction-review-1.md`, `text-conformance-owner-design-1.md`.
- `target/assimp-comparison/convergence-linux-text-emission-symbols/{source.tar,build.json,build.log,baozi-probe}`.
- `target/review/convergence/text-emission-attribution/candidate-*`, `text-emission-attribution-main-summary-1.json`.

## Source disposition

Main accepts the independent source-cleanup recommendation for `7900255` after raw evidence review. The successful immutable-source conformance owner makes repeated build policy qualification unnecessary; the smaller build navigation Interface keeps actual emitted-output and live control checks separate. This source rationale does not require falsely classifying ordinary observations as a performance win. The six small instruction reductions and mixed ordinary results leave the speed hypothesis unproven.

Retain the two Windows grid gains alongside repeated OBJ pipeline costs, shared-glTF/point extraction regressions, worse candidate maxima, parent elapsed observations, whole-child memory limits and asymmetric GNU load/missing confirmation. No negative endpoint is removed because the source change is outside its clock. Source cleanup, measured performance retention and final release qualification are separate conclusions.

Independent disposition is recorded in `.git/baozi-review/performance-convergence/text-emission-native-disposition-review-1.md`; raw and document review is recorded in `text-emission-native-raw-review-1.md` in the same directory. All original convergence/release gates stay open. The next finite geometry owner requires its own reviewed plan, implementation/cleanup qualification, work/cancellation evidence and unchanged ordinary comparison before performance retention.
