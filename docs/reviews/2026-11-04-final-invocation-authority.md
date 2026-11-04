# Final Invocation Authority After Body Cleanup

This is the connected capability removal from the
[lexical invocation-host plan](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md),
following the [success-only joint](2026-11-04-canonical-joint-success.md). Implementation is in
the isolated compiler workspace. The final frozen source passes whole-workspace runtime,
affected compiler documentation, independent consumer, strict lint and formatting checks.
Independent source and final evidence reviews are complete without an actionable finding.
No performance or main Rust application follows from this checkpoint.

## Actual Capability Boundary

The old `CanonicalInvocationHost::split` and `CanonicalInvocationParts` are deleted, including
legacy fixtures. `run_body` lends `CanonicalBodyParts` with only startup, output, proof,
activation and validation permissions. Generation and final failure-fallback slots are issued
only after body/capture cleanup. Central validation's existing internal fallback slot remains
private and retains the actual unprepared fallback.

`CanonicalBodyStart` privately contains the existing startup loan. It has no raw conversion,
root extractor, mutable dereference or replacement constructor. `CanonicalBodyOperation`
starts the same fresh/derived retained operation and forwards bounded execution, reservations,
control, phase completion, failure-work qualification, activation and disposition requests.
It exposes no direct terminal preparation, publication assembly or recoverable raw owner.
Physical observation returns copied resource facts, without lending the account or release
authority. Ordinary raw publication remains its existing distinct API for real ordinary users.

`CanonicalInvocationFinish::resume` restores actual deferred state from the original host and
wraps each of the six canonical branches in `CanonicalFinal<S>`. Only concrete final-state
implementations expose freeze issuance, failure-evidence construction, rich-failure validation
or minimal preparation. Moving a wrapper preserves its particular state; its constructor and
field are private outside activation, with no public extraction, mapper or mutable access.
Final-origin generation rejection returns the corresponding wrapped frozen failure descendant.
The unopened-operation branch retains its sealed request and private operation completion.

These are permission views over the existing actual owners. No account, coordinator, ledger,
domain, execution backend, allocation or numerical algorithm is added. The outer retained
`FnMut` and captures still end before final progress, generation and commit. True parser and
authoring callbacks retain their bounded results and borrowed inputs.

Core's production same-account edit constructor now accepts nominal body startup and returns
the restricted derived operation. Import and both Postprocess entries use the same boundary;
only their final preparation helpers accept restored wrappers. Import's accounted session
cell still ends before the disposition request, and its same-root catalog admission precedes
restoration. All format parser entry points and the numerical kernels are unchanged.

## Real Fixtures and Compiler Contracts

Legacy fixtures now execute inside `run_body`, record a real disposition and consume restored
state outside the callback. They do not create final wrappers or retain a test-only `split`.
Startup fixtures distinguish the real ordinary and body constructors without a production
conversion or alternate backend. Existing numerical, control, work-prefix, typed failure,
storage-release and attribution assertions remain.

The new public Rustdoc examples actually construct an account, catalog, plans and host. One
starts, requests, restores and hands off unchanged success. The other seals a producer request,
materializes a canonical root, builds/validates evidence, requests finish, restores the exact
candidate and completes its freeze/joint/handoff. Each has an adjacent same-construction
negative whose forbidden body expression must fail. Independent probes cover raw startup,
direct preparation, both failure builders, rich-failure validation, all body fallback states,
private final fields/construction/replacement, final-slot absence, deleted `split`, and separate
operation/candidate lifetime escape.

Legal `mem::forget` remains runtime-tested. Forgotten body state with no disposition is
cleaned and contained; unresolved charges are not reported as verified refunds. Forgotten
finish and restored final candidate cases retain actual borrowed payloads in the host, across
all destructor-panic masks. Existing tests retain original-unwind precedence, exact account
and control causes, dropped/forgotten joint behavior and catalog-abort regression coverage.

Three migrated fixtures initially kept old cleanup expectations after abandoning their body
without a disposition. Their original failures are retained. The Import pending-drop,
foreign-root healthy companion, and materialization-capture cases now record the actual
candidate and resume/drop it outside the body, retaining their original healthy-account and
zero-allocation assertions. A distinct unresolved-body test explicitly requires containment,
one payload cleanup, retained physical attribution and no further mutation on host destruction.

## Frozen Source and Qualification

The complete source4 has 1,412 files, including the new body/final views and constructive
Markdown examples. Its source-set SHA-256 is
`2300bd1dbac9527330c7e3d445dcc5249405815e7e6fc19a4165959e76533e96`.
The manifest and serial QA receipt are
`target/review/convergence/final-authority-development-source-4.json` and
`final-authority-development-qa-4.json`; frozen trees and independent reviews are under
`.git/baozi-review/performance-convergence/`. Set encoding is sorted workspace-relative UTF-8
path + NUL + lowercase SHA-256 + LF.

The exact source4 results are:

| Check | Result |
| --- | --- |
| Whole-workspace nextest, all features | **3,469 passed**, 6 existing skips, 151 binaries; run `c7946982-40e6-4cc7-b036-4f8865db0abb` |
| Operation/Core/Import/Postprocess doctests | **142 passed**, no failures or skips |
| Independent asset consumer nextest | **138 passed**, 1 existing skip; run `4bc9e701-dc2b-4ed2-9899-4b8a77b41212` |
| Workspace and consumer Clippy | All targets/features, `-D warnings`, both passed |
| Workspace and exact consumer formatting | Both check commands passed |

All commands use Rust 1.95.0, locked/offline dependency resolution and serialized one-job
builds/tests. The runtime log also contains a separate one-test filtered subprocess result;
it is not added to the 3,469-test nextest total. The initial formatter is preparatory. Its two
complete maps also happen to equal source4; the following seven verification commands have
all **14** complete maps equal to source4. All eight command log hashes match their receipt.
The complete live and frozen trees have exactly the same 1,412 files and hashes.

| Artifact | SHA-256 |
| --- | --- |
| Source4 manifest | `f3bfe1d475b87026b6669073c6630aa2ff7b9b4378f2257e682023d9cd6a4ed6` |
| Source4 serial QA4 | `ad87830031b53765461aa11c8b7f4e806563cd6ace09248674817310229315d4` |
| Independent initial production review | `eda48e62ffbe951595f30ca95a18bb799e1e650ca377799125f19317139758cc` |
| Independent source3 compiler/fixture review | `c4c491705bea95a2323621c513f72f06dbf5ea30b25412fdfc7390b22a921f07` |
| Independent source4/compiler supplement | `ef555262aa1f6e95dc0ca772f9c494bfd75127e4226e9b1e3813730f8d9ba8c5` |
| Independent final source4 evidence association | `636b7800f417541916140ef3d484c82f0c45b284e4f6ce923942915811e9b5b7` |

The independent source4 supplement finds no actionable correctness defect and closes the
generic, constructive and independent escape compiler gaps. Source3 differs only by the
absence of a statement-scoped lint annotation on an intentional test-only `mem::forget`;
that historical strict-lint failure is preserved. The independent final evidence review
enumerates and rehashes both source trees, all sixteen command maps and eight logs, verifies
the actual commands and result totals, and confirms the pre-existing skip declarations.
It finds no remaining required source4 qualification gate or evidence-association gap.

Predecessors remain separately associated. Development Operation units pass 573 tests;
the first complete Operation run has 961 passes and two fixture failures before their actual
disposition repairs. Connected callers initially have 1,460 passes, one pending-drop fixture
failure and six existing skips; the repaired Import then passes 471 tests with no skips.
Source1's complete candidate Rustdoc has an actual E0505 failure from an overlong example
completion lifetime. Source2 corrects only the paired Markdown scopes and passes 138 affected
doctests. Later sources add independent lifetime probes and scoped lint annotations.
None of those predecessor results substitutes for source4's actual whole-workspace, compiler,
consumer, strict lint and formatting results above. The three request/resume/drop fixture
repairs now execute within source4's successful whole-workspace run, retaining their original
healthy-account and exact-refund assertions.

## Remaining Work

Borrowed numeric backing must still replace `LocalWork`/`BoundWorkGroup`'s dynamic alias path.
Current-state resource attachment with one start admission, shared-shell-before-state
promotion, exact accounting and superseded-owner deletion also remain open. Main Rust is
unapplied. This component introduces no ordinary timing/memory campaign; the unchanged
Windows/Linux 14-case/110-lane convergence requirements and release remain HOLD.
