# Lend the original edit-error slot through late result projection

Status: bounded design reviewed with its required amendment adopted. Not
implemented. Parser allowance retirement source3 passes all seven gates;
retirement commit 17f74d5 retains its meaningful behavior tests. Independent
retirement association and the prior experiment's final documentation close
before this implementation. Native convergence, main/runtime integration and
release remain HOLD.

## Demonstrated problem and concrete owners

The exact bdc tiny-STL pipeline/memory worker contains one direct prepare::run
to memcpy call with 10,432 inclusive Ir. Independent source-line/operand review
`tiny-prepare-copy-source-lines-review-1.md` identifies its original
Option<EditInvocationError> carrier, initialized to None at prepare.rs:400 and
lent to finish_bound_edit at :403. The actual instruction at 0x73c691 copies
0x28e0 = 10,464 bytes from that entry slot into the inlined finish_preparation
parameter, before result dispatch. The recorded unchanged-success path has None.
The terminal result occupies distinct storage. This is an emitted carrier move,
not duplicated live numerical authority, an allocation or 10,464 instructions.

The full worker records 2,041,865 Ir; this direct 10,432-Ir arc addresses about
0.51% of its instructions. Removable instructions and native savings remain
unknown. No whole-Host rewrite, boxing, unsafe placement, larger stack or changed
public error is justified. Space conversion has the same actual late helper
pattern: its original entry slot is moved into finish_conversion at :168/:183.
This sibling is a real second caller of the same ownership seam, without a
measured memcpy/saving claim for that worker.

## Selected interface and alternatives

Keep each original optional owning error in its concrete entry function. Pass
&mut Option<EditInvocationError> into the existing late helper instead of moving
the complete optional owner. The helper keeps its sole take().expect(...) in
the Err(EditFailure::Invocation) branch, returning the same public owned failure.
Borrowing matches finish_bound_edit's existing interface. Change two call
arguments and two private parameter types; introduce no wrapper, provider,
closed outcome hierarchy, shared generic finalizer or alternate implementation.

| Alternative | Decision |
| --- | --- |
| Original slot loan through existing concrete helper | Selected; removes the demonstrated by-value interface and preserves single take |
| Move the slot or entire Host through another wrapper | Preserves/adds large carrier movement; rejected |
| Box/Arc the error or replace it with a small cause | Changes allocation or loses original owner/evidence; rejected |
| New RAII slot guard / callback framework | Unnecessary for the current direct pairing; do not add hypothetical machinery |
| Space helper left with the same owning parameter | Smaller measured scope but leaves the identical sibling ownership seam; change its private interface consistently, with independent source proof |

## Ownership and reachable-path obligations

Hard implementation gate before accepting the four-line edit: inspect each
actual helper/caller and require Invocation => Some(original_error) and
Unchanged/Changed/Semantic => None, with take() reached before any new fallible
or panic-capable operation in the occupied branch. If either condition changes,
stop and redesign this seam; do not add a guard/framework under this plan.

The current result pairing must be established separately for both helpers:
Err(EditFailure::Invocation) has Some(original_error); every other returned
result has None. Preserve scene_edit's empty-slot assertion, original install,
short-circuit marker propagation and exactly one take. Keep original input,
public failure field order, semantic causes/evidence and final-control policy.
Do not claim unconditional destructor equivalence merely because the type borrows.

The occupied invocation branch reaches take directly, without newly fallible or
panicking projection work beforehand; take empties the slot before owned wrapping.
Other branches have no error owner in the slot. On normal return, the invocation
failure transfers the exact owner and every remaining caller slot is empty.
On projection unwind, distinguish the temporarily transferred error from the
empty slot and preserve public input/error destruction. A future non-invocation
Some(error), or new panicking work before an occupied take, needs a new proof;
do not silently generalize the current invariant.

The early FiniteBodyExitGuard remains armed before entry/capture destruction;
it is not replaced by the later outer optional slot. The actual fixture
finite_returned_error_is_guarded_before_unconsumed_entry_lineage_panics must
retain error-drop zero through entry/capture teardown, exactly one final drop
and the first original lineage panic. Preserve actual admission/domain/root,
capture cleanup, proof/output order and original error-owned Drop. No host,
terminal, numerical counter, resource policy or physical publication changes.

## Scope and qualification

Production scope: crates/baozi-postprocess/src/prepare.rs and space.rs only.
Reuse existing actual unchanged/changed, typed semantic/invocation rejection,
cancellation/final-observer, storage denial, original-input return, capture/
lineage and ordinary-host unwind fixtures plus the six explicit 1 MiB stack
tests. Add a negative test only if source review exposes a missing reachable
owner/error path; no implementation-mirroring slot accessor, layout assertion,
counter probe, blanket suppression or new test framework.

Freeze exact inputs and run affected Postprocess/Operation coverage followed
by the original seven workspace/independent-consumer gates, with independent
source and actual QA/source-commit review. Current repository/existing target,
Rust 1.95, Cargo jobs1 and nextest threads1 remain. No source writer overlaps QA.
Retain all earlier failed candidates, launcher failures and current user notes.

## Prospective measurement and decision

Control is qualified source bdc92b3fa5ca523a99920ff980b909ecfe6d61e9 and its
retained source-bound symbol/ordinary workers. Current retirement 17f74d5 has
exactly that production source; the sole inherited Rust difference is the
parser test file, which is excluded from the uninstrumented release worker.
The five inspected projection/error source files equal the annotated evidence
revision byte-for-byte. Recheck that no other production difference enters the
future isolated source-only candidate. Bind its exact commit, archive, QA and
binary identities before measurement. No different worker's copy claim is used.
Use matched GNU opt3/debug1 System workers for the original six tasks and an
additional fresh pair of the original 134-byte STL1 pipeline/memory task.
This diagnostic addition does not change the original fourteen-case/110-lane
native matrix, fields, work, clocks, precision or IO policy.

Require full literal non-clock equality on all seven pairs. For tiny STL1,
require >=0.25% whole-worker Ir reduction, using exact integers
400*candidate <= 399*control; its demonstrated addressed upper bound is about
0.51%, not a forecast. All original six must retain the unchanged regression
ceiling 100*candidate <= 101*control. These prospective criteria apply only to
the distinct error-slot hypothesis, never to revive the failed parser allowance
or relabel its fixed one-OBJ>=1% result. Inspect retained source-bound codegen to
verify the identified success-path carrier copy is absent. That absence is a
required selection condition in addition to the integer instruction predicate;
an unchanged identified copy fails this hypothesis even if unrelated Ir improves.
Explain any distinct residual copies without treating them as the same owner.
No compiler frontend, new instrumentation or byte/time inference from Ir.

Failure rejects this distinct candidate or separately redesigns its owner seam.
Instruction PASS qualifies further measurement only. Keep original Windows/GNU
14/110, 3 warmups/20 retained fresh AB/BA pairs, separate representative calibrated
heap, independent actual raw evidence and all failures/missing/load/regressions.
Every pipeline <=3x same-campaign Assimp, no confirmed endpoint >=10x and comparable
whole-child memory <=3x remain unchanged full-goal targets. No source-retention,
main/runtime integration or release acceptance follows from diagnostic PASS.
