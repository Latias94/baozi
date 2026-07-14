# ADR Governance

This document is Baozi's living contribution policy for Architecture Decision Records (ADRs). It
governs how decisions are proposed, accepted, amended, superseded, indexed, evidenced, and checked.
It does not define product architecture: each Accepted ADR remains the sole semantic authority for
the concern it owns. This policy is maintained like any other repository contribution policy. It
does not need, and cannot derive authority from, an ADR that attempts to authorize its own
lifecycle.

## Authority Model

Baozi separates four roles:

1. Each `status: accepted` ADR is the semantic authority for its owned architectural decision.
2. The [ADR index](../adr/README.md) is the canonical registry for decision ownership, stable
   requirement-to-authority traceability, titles, and lifecycle status.
3. This document is the ADR process policy.
4. Source code, tests, plans, research, conformance inventories, and progress records are evidence or
   execution state. They never silently override an Accepted ADR.

ADR number, file date, and merge order do not establish precedence between overlapping Accepted
ADRs. The ownership registry must identify one owner, or an Accepted replacement must explicitly
supersede the old decision. Plans may schedule work but cannot acquire architecture authority.

Decision authority, implementation conformance, format maturity, and publication readiness are
independent axes:

| Axis | Canonical record | Meaning |
| --- | --- | --- |
| Decision authority | ADR status plus the ADR index | which architectural contract governs work |
| Implementation conformance | repository conformance inventory, direct tracers, and executable gates | whether current code satisfies an Accepted contract |
| Format maturity | ADR 0011 plus format evidence | how much one exact format profile has proved and who verified it |
| Publication readiness | ADR 0006 plus release/package gates | whether artifacts may be published and supported |

An Accepted ADR may therefore be unassessed, partially implemented, conforming, or nonconforming.
Implementation lag does not by itself change the ADR back to Proposed. A known disagreement is
recorded as partial or nonconforming until code changes or an Accepted replacement changes authority.

## Lifecycle

```mermaid
stateDiagram-v2
    [*] --> Proposed
    Proposed --> Proposed: isolated controlled-trial evidence
    Proposed --> Accepted: reviewed decision approval
    Proposed --> Rejected: candidate declined
    Accepted --> Accepted: compatible amendment
    Accepted --> Superseded: Accepted replacement
    Superseded --> [*]
    Rejected --> [*]
```

The only lifecycle values are `proposed`, `accepted`, `superseded`, and `rejected`:

- Proposed records a candidate and grants no production, default, publication, or compatibility
  authority.
- Accepted is normative for the concern assigned to it by the index.
- Superseded and Rejected are historical records and own no active concern.
- Trial is not a lifecycle value. It is isolated evidence-gathering work under a Proposed ADR.

An Accepted ADR becomes Superseded, never Rejected. Historical files remain in place so links and
rationale remain reviewable.

ADR IDs are permanent, monotonically assigned labels rather than a dense sequence. IDs are never
reused and later ADRs are never renumbered to close a gap. A genuine decision that entered reviewed
history remains as Rejected or Superseded; an erroneous pre-release record that never represented an
architectural decision may be removed during baseline cleanup, leaving its ID intentionally unused.
Every such ID is recorded in the [retired-ID registry](../adr/README.md#retired-adr-ids). That registry
is identifier governance, not an ADR or semantic owner. The checker contract requires active ADR IDs
and retired IDs to be disjoint; every ID from 0001 through the maximum active-or-retired ID belongs
to exactly one set; retired entries are unique, four-digit, numerically sorted, use the canonical
`Permanently unassigned` disposition, and carry a nonempty reason; and a newly assigned ID is greater
than that maximum.

## Amendments And Supersession

An Accepted ADR may be amended in place only when it keeps the same canonical concern, seam, selected
architectural direction, and load-bearing guarantees. Clarifying an invariant, closing an ambiguity,
or strengthening evidence/security without invalidating the original rationale is normally an
amendment.

A replacement ADR is required when a change:

- moves, splits, or merges canonical ownership;
- selects an alternative the ADR rejected or materially changes its primary seam;
- removes or reverses a load-bearing guarantee;
- makes the original rationale, alternatives, consequences, or compatibility impact materially
  false; or
- needs both old and new contracts to remain understandable as distinct decisions.

Every authority-affecting amendment updates `updated` and appends an `## Amendment History` entry.
The entry records the date, affected headings, previous rule, new rule, compatibility/security
impact, review evidence, and why amendment is appropriate instead of supersession. Authors are not
implicitly approvers. Editorial changes that do not alter normative meaning need neither `updated`
nor an amendment entry.

Template:

```markdown
## Amendment History

### YYYY-MM-DD: Short description

- Affected: `Decision / ...`
- Previous: ...
- New: ...
- Impact: ...
- Review: pull request, commit, or named review record
- Rationale: why this refines rather than replaces the decision
```

### Initial Pre-Release Baseline

The reviewed change that first adds this policy establishes Baozi's initial pre-release ADR
baseline. Authority-affecting wording already present in that same change does not require
reconstructed amendment entries or retroactive approver claims. Existing `updated` dates remain
historical metadata. Every later authority-affecting amendment must follow the amendment rule above.

Supersession is atomic and reciprocal. A Superseded ADR lists one or more Accepted replacements in
`superseded_by`; every replacement lists the historical ADR in `supersedes`. Proposed and Rejected
ADRs cannot declare supersession authority. The ownership registry changes in the same reviewed
change.

## Frontmatter

Every numbered ADR uses the strict frontmatter subset below. Scalars use `field: value`. Lists use a
two-space block list or the literal `[]`. Unknown fields, duplicate keys, invalid null aliases, and
duplicate normalized paths fail closed.

| Field | Cardinality | Meaning |
| --- | --- | --- |
| `status` | Required scalar | One legal lifecycle value. |
| `date` | Required scalar | Original decision date in `YYYY-MM-DD`. |
| `updated` | Optional scalar | Latest authority-affecting amendment date; cannot precede `date`. |
| `authors` | Required non-empty list | Decision authors, never inferred approvers. |
| `decision_type` | Required scalar | Stable navigation classification. |
| `depends_on` | Optional list | Normative ADR dependencies. Accepted ADRs depend only on Accepted ADRs. |
| `related` | Optional list | Non-normative navigation with no precedence or authority implication. |
| `supersedes` | Optional list | Accepted ADRs replaced by this Accepted ADR. |
| `superseded_by` | Optional list | Accepted replacements for this historical ADR. |
| `evidence` | Optional list | Durable repository-local facts, measurements, or conformance artifacts. |
| `tracers` | Optional list | Exact repository files directly implementing or executing the decision. |
| `promotion_requires` | Optional list | Stable promotion gate IDs resolved through the repository gate catalog. |

`depends_on` is the only dependency field that affects authority, and its graph must be acyclic.
`related` is navigation only and cannot make a Proposed ADR normative.

## Evidence And Tracers

Evidence explains why a decision is credible. A tracer is stronger: it names one exact repository
regular file that directly implements the decision or executes an assertion through a production,
test, or CI path.

Directories, globs, plans, placeholders, prose status notes, generated transient output, and
unrelated files are not tracers. Every tracer, regardless of ADR status, must resolve with exact
casing to a regular file that is tracked or staged, is not Git-ignored, and crosses no symbolic-link,
junction, reparse, `.git`, or `repo-ref` component. Structural validity is a prerequisite, not proof
that the tracer demonstrates the claimed behavior; review and executable gates establish directness.

Repository-local evidence must exist in a clean checkout and obey the same path-containment rules.
An evidence file may be new and unstaged while a complete change is being prepared; a promotion
tracer must already be tracked or staged at the promotion boundary.

External evidence is written in ADR prose, never in `tracers`, and uses a reproducible identity:

- source/history uses a full commit object ID rather than a branch, mutable tag, or release page;
- registry packages use an exact version and content checksum;
- immutable specifications use an exact edition/version and official versioned URL; and
- a living official page without an immutable edition records its canonical URL, retrieval date,
  final response-body byte length, and SHA-256 digest after redirects.

A floating URL without that capture is navigation, not acceptance or promotion evidence. ADR
Markdown uses inline links so every destination passes through the same path validation.

## Controlled Trials And Promotion

A Proposed ADR may reference an isolated trial implementation, tests, and measurements. Trial code:

- is unreachable from production defaults and released feature sets;
- creates no support, compatibility, or publication promise;
- cannot make another Accepted ADR conform to the proposal; and
- remains removable if the proposal is rejected.

Promotion is one atomic reviewed change containing the decision, implementation state, exact tracer
metadata, gate evidence, index update, and `status: accepted` transition. An implementation may be
developed before acceptance; its authority changes only at promotion.

Promotion requirements are stable gate IDs, not ADR-number special cases. The gate catalog defines
the implementation, test, and CI tracer classes plus the executable command or workflow that supplies
passing evidence. An unknown gate, absent required tracer class, or missing passing result blocks
promotion. Tracer presence alone never proves a gate passed.

ADR 0037 and ADR 0038 retain `implementation_tracer` while they remain deferred proposals. The
current checker also contains a legacy unconditional guard for those IDs. That hard-coded branch is
an implementation-conformance debt: the policy is the generic gate model, and removing the branch
requires a later checker change rather than another governance ADR.

## Ownership Registry

The [ADR index decision-ownership table](../adr/README.md#decision-ownership) is the only handwritten
concern-to-owner registry. It assigns ownership but does not restate semantic rules. Moving,
splitting, merging, or intentionally removing ownership updates the registry and affected ADRs in one
reviewed change.

The index also keeps stable requirement-to-authority traceability. Delivery units, milestones, and
progress remain in plans. Generated reports may mirror the registry, but no second handwritten owner
catalog is allowed.

## Conformance Inventory

The architecture conformance inventory is separate from ADR status. Its minimum record contains:

- stable `contract_id` and one Accepted `authority_adr`;
- `conformance_status`: `unassessed`, `partial`, `conforming`, or `nonconforming`;
- separate tracked implementation, test, and CI tracers; and
- one or more stable gate IDs resolved through the executable gate catalog.

`conforming` requires every tracer class applicable to the contract plus at least one passing
executable gate. Missing files, unknown gates, non-Accepted authority, or self-reported status without
gate evidence fail closed. Format maturity and publication readiness remain in their own inventories
and may reference the same stable contract IDs.

Until that inventory exists, plans and progress records may locate incomplete work but cannot claim
repository-wide conformance, format maturity, or release readiness.

## Automated Enforcement

The canonical local structural command is:

```console
cargo run --locked -p baozi-xtask -- adr-check
```

Because ID permanence is historical, this gate fails outside a Git worktree or in a shallow clone.
Repository CI checks out the complete current-`HEAD` ancestry before running it.

The checker contract requires it to verify:

- unique canonical ADR filenames; a unique, normalized, sorted retired registry; disjoint and
  exhaustive active/retired ID sets through the current maximum; permanence and monotonic new IDs
  backed by the complete current-`HEAD` ancestry, including merged side-branch history but excluding
  unrelated refs; and one exact index row per active ADR;
- filename, heading, title, status, and authority-column agreement;
- the frontmatter schema, normalized duplicate paths, Accepted dependency closure, and acyclic
  `depends_on` relationships;
- reciprocal legal supersession;
- local link/fence/fragment structure and exact-case contained paths; and
- implemented promotion/tracer prerequisites.

The checker cannot infer whether prose competes with another ADR, whether a tracer directly proves a
behavior, whether an amendment should have been a supersession, or whether architecture is complete.
Those are review responsibilities. Documentation must not claim that structural validation proves
semantic coherence. Current implementation conformance and scheduled checker work are tracked
outside this policy; implementation lag does not weaken this contract.

## Change Checklist

- The ADR answers one architectural decision and includes at least two genuine alternatives.
- Architecture flow or state relationships have a Mermaid diagram.
- Risks have concrete mitigations and success criteria are measurable.
- The owner registry and stable requirement traceability identify the correct Accepted authority.
- Normative dependencies are Accepted and use `depends_on`; navigation uses `related`.
- Every local evidence/tracer path is durable, exact-case, and directly relevant.
- An Accepted amendment has `updated` plus amendment history, or a replacement ADR uses reciprocal
  supersession.
- Promotion gates and conformance claims have executable evidence rather than file-presence claims.
- `adr-check` passes in a clean-checkout-equivalent repository state.
