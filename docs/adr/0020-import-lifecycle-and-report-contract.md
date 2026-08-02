---
status: accepted
date: 2026-07-09
updated: 2026-08-01
authors:
  - Codex
decision_type: architecture
depends_on:
  - docs/adr/0002-runtime-concurrency-observability.md
  - docs/adr/0003-core-scene-ir-and-material-model.md
  - docs/adr/0010-asset-io-virtual-filesystem-uri-archive-and-path-security.md
  - docs/adr/0016-import-options-presets-and-configuration-precedence.md
  - docs/adr/0021-extension-registry-and-format-descriptor-governance.md
  - docs/adr/0022-per-import-resource-ledger-and-budget-accounting.md
  - docs/adr/0029-format-detection-stable-input-and-failure-contract.md
  - docs/adr/0033-operation-control-and-observability.md
  - docs/adr/0036-source-identity-provenance-and-metadata.md
  - docs/adr/0039-source-document-scene-selection-and-import-closure.md
  - docs/adr/0041-resource-dependency-graph-and-successful-input-manifest.md
  - docs/adr/0044-retained-physical-storage-ownership-and-account-lineage.md
related:
  - docs/adr/0013-post-process-pipeline-semantics-presets-and-mutation-model.md
  - docs/adr/0014-parser-security-unsafe-ffi-and-panic-boundary-policy.md
  - docs/adr/0025-scene-ir-invariants-and-validation-boundaries.md
  - docs/adr/0043-producer-attestation-closed-input-qualification-and-import-build-identity.md
---

# ADR 0020: Import Lifecycle, ParseScope, and Outcome Contract

## Context

If a parser can open arbitrary resources, construct a report, assign a lifecycle enum, or declare its
own Scene valid without the central core validator, facade helpers cannot enforce one security and
correctness boundary. Creating context only after opening/detection also means early failures lose
diagnostics, resource evidence, and a single finalization path.

Older documents used conceptual names such as RawImported, ValidatedImported, PostProcessed, and
ValidatedOutput. This ADR is the sole authority for import lifecycle and import outcomes. It does not
own post-process internal stages, which belong to the post-process contract, and a validated Scene
proof is not an assignable lifecycle stage.

## Decision

After request/options compilation, Baozi performs one fallible session-bootstrap transition before
primary binding or detection. Only a successful bootstrap creates `ImportSession`.
The session owns the only outcome builder, stable primary input, selected-format evidence, optional
source-scope evidence, dependency resolver, ledger, bounded diagnostics, and operation control. It
also creates and retains exactly one opaque core `SceneConstruction` bound to that existing session
account, ledger, control, and publication context. It grants the selected trusted importer only a
capability-limited `ParseScope` that borrows the construction's bounded authoring surface.

The parser returns `ParseResult<SceneDraft>`. A draft contains only its core construction brand and
canonical directory-slot descriptor; it does not own a separately mutable Scene or a generation
identity. The parser cannot move, replace, inspect, or finalize the construction owner and cannot
reach its retained owner, account, ledger, validation, publication, or generation authority.
`baozi-core` is the only constructor of `ValidatedScene` proof; standalone callers may drive a public
core construction entry point, but no API accepts a draft descriptor without its matching owner and
no caller can construct or forge proof directly.
Successful preparation reserves ADR 0003's private `ReservedSceneGeneration` as its final fallible
pre-freeze identity acquisition. Reservation rejection or exhaustion consumes no nonce and
is classified during preparation as an ordinary rich failure with preserved evidence and zero
canonical output. It uses minimal fallback only if appending, validating, or preparing that rich
failure itself then fails. No `SceneGenerationId` exists yet.
`ValidatedScene` is constructible only when pure handoff consumes a successful reservation into the
public generation ID.
`ImportSession` is the only import outcome/stage/provenance finalizer. On parser success it consumes
the retained construction together with the exact returned descriptor. A missing, duplicated,
already-consumed, or foreign descriptor is a trusted-extension contract failure and cannot publish a
Scene. A core construction failure is latched before control returns to format code and remains
authoritative even if the parser catches its error or returns another value. After bootstrap succeeds,
the session returns exactly one terminal API result. Safely finalizable paths return one
`ImportReport` or `ImportFailure`. If the process survives
but storage/dependency release cannot be verified, it instead returns one fixed typed
`FinalizationContainmentError`, keeps uncertain payload/lease/operation attribution coupled and
charged, and publishes neither report nor fallback.

Core owns canonical validation, proof construction, generation reservation, and canonical handoff.
`ImportSession` owns import phase/provenance, successful-manifest ordering, report/failure evidence,
and the single public import terminal. Import composition uses a sealed pre-handoff boundary so the
canonical and report-evidence tables freeze in one outcome; it never creates an earlier independently
published `ValidatedScene`.

### Lifecycle

Compiling and validating the complete common and format-option request is a pre-session admission step.
An invalid request returns a typed `RequestError`, creates no session, and performs no asset IO. After
successful admission, the import lifecycle is:

During request compilation, an unwindable submitted-layer codec panic, compile-budget exhaustion,
runtime option-type mismatch, or other codec/request defect is a bounded `RequestError`, never an
`ImportFailure`; default-codec failures occurred earlier as `RegistryBuildError` and cannot exist in
the frozen snapshot. Neither path has an import ledger, lifecycle phase, diagnostics stream, finalizer,
or asset open.

`ImportSession::try_start` then obtains either a host-supplied physical-account capability or creates
a private account through ADR 0044's explicit host-owned account factory, creates the real operation
ledger, and preflights the complete minimal-terminal aggregate. A private two-phase startup guard
first admits the complete operation-attribution aggregate under the ledger's one lock, releases that
lock, and then admits the complete physical aggregate under the account's one lock. The locks are
never nested, and the two internal snapshots do not share a linearization point. If the second
admission returns an ordinary typed rejection that proves no physical mutation, the guard
structurally rolls back the first aggregate before returning `SessionStartError`. If that transition
panics, observes poison after mutation, or cannot prove charge/owner disposition, it instead preserves
the provisional attribution coupled with every possible physical owner, poisons the account/path,
and follows ADR 0044's fail-closed startup-containment result without refund or an accepted session.
Only the ready pair may enter the first retained allocator and fallibly
allocate/bind the envelope. The session remains externally unaccepted until both planes succeed, so
session acceptance is atomic even though the two accounting planes remain independently linearized.
Its fixed `FallbackHeader<P>` carries `schema_revision`, Baozi-owned inert
`terminal_cause`, optional `last_completed_phase`, `dimension_count`, `phase_count`, and the
operation-wide `current_live_bytes`, `peak_live_bytes`, `active_reservations`,
`current_evidence_bytes`, and `peak_evidence_bytes`. The global fields are stored facts; neither the
per-dimension nor per-phase cells can reconstruct the operation-wide peaks or evidence-storage
high-water mark. Successful preparation verifies `current_live_bytes == 0`,
`active_reservations == 0`, and `current_evidence_bytes == 0` while preserving both peak values.

The dynamic evidence uses sealed `FallbackDimensionCell<D> { key, snapshot }` and
`FallbackPhaseCell<P> { key, snapshot }` values. Each key is a fixed-size copied Baozi operation-
domain value with no `Drop`, external object, trait object/vtable, function pointer, or executable
plugin state. `StoreKeyDomain` is a process-local minting authority, not stable semantic identity.
The Baozi-owned result/evidence owner structurally retains that domain's lifetime anchor and complete
immutable normalized key mapping until the last supported query ends. A plugin receives only keys
minted by the host for its admitted declarations. It may create an unrelated process-local domain,
but cannot copy or access the host authority, mint an accepted host-domain key, inject a raw key, or
own the retained mapping; every foreign-domain key is rejected. Normalization completes synchronously
before the first ledger entry, and failure rejects the entry. Query after plugin unload uses only the
copied key and owner-held mapping and cannot call or retain the plugin. The process-local brand, its
raw value, and its allocation order enter no semantic
digest, snapshot transcript, wire format, persistent cache key, or build identity; stable meaning
comes only from the Baozi-owned normalized catalog.

Bootstrap checked-converts both counts, builds
`Layout::array::<FallbackDimensionCell<D>>(dimension_count)` and
`Layout::array::<FallbackPhaseCell<P>>(phase_count)`, extends those semantic layouts after
`Layout::new::<FallbackHeader<P>>()` in that order, and calls `pad_to_align`. Every array accounts for
the complete key, snapshot, alignment, and per-cell padding. The resulting complete layout is an
aggregate shape/overflow preflight, not a promise of one allocator backing or a stable ABI. The fixed
header owner and one or more keyed-evidence backings may be separate allocations; every actual
allocation has its own policy-governed `CapacityPlan` and layout/capacity charge. A separate source
evidence anchor pairs the concrete-store and capacity-policy revisions. Before the first
allocator call, bootstrap checked-sums every requested envelope and block count, admits the complete
attribution aggregate in one ledger transition, and then admits the complete physical aggregate in
one account transition through the private guard described above. A future single-backing
implementation must preserve the same observable admission, rollback, evidence, query, and failure
contract without merging the two locks or accounting snapshots.

Session startup delegates construction and containment of its fallback/evidence storage to ADR 0044.
The import lifecycle accepts a session only after that storage has reached the exact startup-baseline
state required by later terminal downgrade. Every failed startup maps to `SessionStartError` with no
accepted session, report, failure, or fallback publication. This ADR owns that lifecycle mapping; it
does not redefine the physical side, attribution, cleanup-evidence, or membership algorithm.

Any count conversion, multiplication, addition, array, extend, aggregate-envelope sum, block-count,
or actual-allocation layout overflow rejects bootstrap before a session or risky work exists; no
dimension-only formula or aggregate-layout-as-single-allocation shortcut is permitted. Failure before
all of those owners exist returns fixed-size `SessionStartError`. That error
has no heap allocation, account lease, ledger snapshot, lifecycle phase, diagnostic stream, or asset
open; it distinguishes invalid/bootstrap limits, account-control admission/allocation, operation-
ledger start, terminal-envelope admission, and terminal-envelope allocation. It is neither
`RequestError` nor `ImportFailure`, and formatting it requires no allocation.

A retained startup builder panic maps to `SessionStartError` and creates no session or import
terminal. A retained candidate/report builder panic after successful startup maps to
`FinalizationContainmentError` and cannot publish minimal fallback. ADR 0044 owns the physical
containment facts preserved by either mapping.

For an explicit account, `try_start` first performs ADR 0044's operation-start admission under the
shared physical-account poison/counter linearization state. Poison-first returns allocation-free
typed `SessionStartError::AccountPoisoned` before an import ledger, work/counter charge, asset IO,
retained-share/refcount access, or allocator call. Every account clone observes the same latch; no
import/facade path can reset, ignore, or replace it. A private account starts with a fresh unpoisoned
state. If start or one retain admission linearizes before another operation poisons the shared
account, its charge remains coupled to storage, all later retain/share admissions reject, and this
session must reach fatal containment during final preparation rather than publish a report/failure.

Low-level session APIs receive an explicit bounded account or factory. The common facade runtime is
constructed from a finite documented resource profile, retains one bounded factory for its lifetime,
and asks it for a private account per import without exposing physical-account or ledger types.
Advanced hosts may deliberately supply one explicit account as an aggregate retention domain. Neither
facade path creates a process-global or per-request factory, and account sharing alone never grants
canonical inheritance between independent Scenes.

1. successfully bootstrap the session and private outcome builder with one pre-admitted account-bound
   minimal terminal envelope, or return `SessionStartError` before a session exists;
2. bind one immutable primary snapshot and its session-issued root-origin capability through either
   policy-authorized IO acquisition or admitted direct-byte copying;
3. build one shared probe and select/force a format with evidence;
4. create exactly one session-bound core `SceneConstruction`, retain its opaque owner in the session,
   and create `ParseScope` for only the selected importer with a borrowed bounded authoring surface;
5. acquire required/optional dependencies through session-issued origin-qualified capabilities and
   parse into format-private state as needed;
6. construct a Scene draft only through that borrowed authoring surface and receive only the
   descriptor issued by the session-bound construction;
7. have `ImportSession` consume its retained construction with the exact returned descriptor, reject
   missing/foreign/latched-failure state, and run central `baozi-core` validation without minting a
   generation or exposing a `ValidatedScene`;
8. reserve one private never-reused generation nonce as the final fallible pre-freeze identity
   acquisition,
   finish final control classification as exact `PreparedSuccess`, apply canonical/outcome
   destination commitments with zero operation-live attribution, and purely hand it off by consuming the reservation into the
   `SceneGenerationId` carried by the report's unforgeable `ValidatedScene` proof; or
9. from any point after successful session bootstrap, prepare and infallibly hand off one
   exact `PreparedRichFailure` or `PreparedMinimalFallback`, each structurally containing no
   generation reservation or canonical attribution; or
10. when the process survives but finalizer cleanup/release cannot be proved, conservatively retain
    coupled ownership/charge, atomically latch the shared physical account and mark the operation/
    extension path poisoned, and return `FinalizationContainmentError`
    without preparing or publishing a fallback.

Last-completed phase is a sealed read-only evidence value constructed by the session. It is present so
callers can understand a failure, not so callers or parsers can advance state. Public code may obtain
`ValidatedScene` only through a matching core construction or validated edit transaction; it cannot
directly construct proof or attach import phase/provenance to it. Only the session can construct
import phase evidence, reports, or failures.

```mermaid
stateDiagram-v2
    [*] --> CompileRequest
    CompileRequest --> RequestError
    CompileRequest --> StartSession
    StartSession --> SessionStartError
    StartSession --> SessionCreated
    SessionCreated --> PrimaryBound
    PrimaryBound --> SelectionCompleted
    SelectionCompleted --> ResourceGraphActive
    ResourceGraphActive --> RetainedBuildActive
    RetainedBuildActive --> DraftBuilt
    DraftBuilt --> DraftValidated
    DraftValidated --> InputManifestFinalized
    DraftValidated --> PrepareRichFailure: manifest/finalization failure
    InputManifestFinalized --> GenerationReserved: final fallible pre-freeze identity acquisition
    GenerationReserved --> PreparedSuccess: commit destinations; live = 0
    GenerationReserved --> PrepareRichFailure: later downgrade; burn reserved gap
    PreparedSuccess --> ImportReport: consume reservation; pure split
    SessionCreated --> PrepareRichFailure
    PrimaryBound --> PrepareRichFailure
    SelectionCompleted --> PrepareRichFailure
    ResourceGraphActive --> PrepareRichFailure
    DraftBuilt --> PrepareRichFailure
    InputManifestFinalized --> PrepareRichFailure: reservation rejection/exhaustion; consume no nonce
    PrepareRichFailure --> PreparedRichFailure: append/validate/prepare succeeds; live = 0
    PrepareRichFailure --> PreparedMinimalFallback: rich preparation itself fails
    RetainedBuildActive --> PreparedMinimalFallback: non-builder unwind; candidate cleanup verified; baseline intact
    DraftBuilt --> PreparedMinimalFallback: invalid root; candidate cleanup verified; baseline intact
    PrepareRichFailure --> FinalizationContainmentError: cleanup/release proof unavailable
    RetainedBuildActive --> FinalizationContainmentError: retained builder panic or side containment
    PrepareRichFailure --> FinalizationContainmentError: retained evidence builder panic or side containment
    PreparedRichFailure --> ImportFailure: pure consuming split
    PreparedMinimalFallback --> ImportFailure: pure consuming split
    ImportReport --> [*]
    ImportFailure --> [*]
    FinalizationContainmentError --> [*]
    RequestError --> [*]
    SessionStartError --> [*]
```

The state diagram is internal authority flow, not a public mutable state machine.

### Phase Types and Mapping

Three sealed phase vocabularies are intentionally distinct:

- `ImportPhaseEvidence` is the coarse authority boundary retained as `last_completed` in an outcome;
- `ImportLedgerPhase` attributes current/high-water reservations and attempted/consumed work; and
- `ImportProgressPhase` labels transient observer events.

The session owns all three import vocabularies. `baozi-operation` stores owner-supplied phase keys but
does not define their meaning. A ledger transition or progress event can never advance lifecycle
evidence. Only successful completion of an atomic session boundary performs that advancement:

| Work underway | Last completed `ImportPhaseEvidence` | `ImportLedgerPhase` examples | `ImportProgressPhase` examples |
| --- | --- | --- | --- |
| Session created, primary not yet bound | `SessionCreated` | `PrimarySnapshot` | `AcquirePrimary` |
| Primary complete, selection underway | `PrimaryBound` | `Detection` | `Detect` |
| Parser/dependency/backend/normalization/build work | `SelectionCompleted` | `DependencyAcquisition`, `BackendDecode`, `FormatNormalization`, `SceneConstruction` | `AcquireDependency`, `Decode`, `Normalize`, `BuildDraft` |
| Draft complete, validation underway | `DraftBuilt` | `Validation` | `Validate` |
| Validation complete, outcome finalizing | `Validated` | `OutcomeFinalization` | `Finalize` |

Formats without a distinct backend or normalized model simply never enter those ledger/progress
phases; they do not fabricate lifecycle milestones. Failure retains the last boundary completed before
the failing work. Outcome finalization does not create a later public phase because the report or
failure itself proves finalization.

### DetectionScope Capabilities

Every detector invocation receives the same immutable `Probe` and hint plus a session-created
`DetectionScope`. Each invocation receives a fresh scope view over the session's shared coordinator
and terminal latch. The scope exposes only cancellation/deadline polling and linear detection-work
admission; it cannot open resources, mutate detection evidence, inspect another registration, or
construct an outcome/failure category.

`DetectionScope::begin_work(amount)` first verifies that the session is active, rejects a second
outstanding permit, polls operation control, rejects zero or more than 4 KiB of work, and charges
attempted `DetectionWork`. Exactly one linear permit represents that admitted chunk. Calling
`complete()` charges consumed work before a post-work control poll. Polling while a permit is live,
requesting zero work, requesting another permit, dropping an unfinished permit, or leaving a permit
outstanding when the detector returns is a descriptor-contract violation and latches the session.

The first control, ledger, reentrancy, or permit-contract failure is retained as the terminal scope
cause. `DetectionAbort` is opaque and non-constructible by extensions. Catching or discarding it and
returning a match cannot recover the session: the session checks the latch before accepting the
detector result, and an abort without a latched cause is itself a descriptor-contract failure.

### Registered Importer Invocation Contract

This ADR owns the exact detector/parser invocation shapes used after ADR 0021 registration and
request admission:

```rust
pub type DetectionResult<T> = Result<T, DetectionAbort>;
pub type ParseResult<T> = Result<T, ParseFailure>;

pub trait FormatImporter: Send + Sync + RefUnwindSafe + 'static {
    fn detect(
        &self,
        probe: &Probe<'_>,
        hint: &ReadHint,
        scope: &DetectionScope<'_>,
    ) -> DetectionResult<DetectionOutcome>;

    fn parse(&self, scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft>;
}
```

Descriptor and option-codec access may use separate registry-owned views, so auxiliary methods need
not match this sketch. The two domain return shapes and borrowed scope capabilities are normative.
Detection cannot return a bare outcome that bypasses its latch, and parse cannot return a backend
error, `ValidatedScene`, validation proof, report, or arbitrary associated error. The registry health
wrapper passes these invocation values unchanged and may observe unwind versus return, but cannot
inspect or relabel the domain result.

### ParseScope Capabilities

`ParseScope` is valid only for the selected importer and session lifetime. It may expose:

- the immutable primary snapshot, normalized source identity, and opaque session-issued root origin;
- classified required/optional dependency requests through the session resolver, using only an
  origin capability returned by this session;
- the selected importer's checked concrete option view;
- checked work charging and live reservations;
- bounded diagnostic emission and loss facts;
- one sealed, bounded source-scope evidence submission;
- cancellation polling and progress reporting; and
- a borrowed bounded Scene-authoring capability whose output is only the session construction's
  `SceneDraft` descriptor and which cannot move, replace, inspect, or finalize the owner or mint or
  submit validation proof.

It does not expose raw `AssetIo`, unrestricted path resolution, registry mutation, another format's
options, resource-graph/cache internals, graph-node construction, caller-selected base paths,
manifest or proposed identity builders, the outcome builder, report/failure constructors, lifecycle
advancement, validation-proof constructors, or post-process execution.

### Source-Scope Evidence Transport

This ADR consumes ADR 0039's semantic `SourceScopeRecord` through one fallible ledger-charged builder,
one sealed `ParseScope` submission capability, and atomic storage in both `ImportReport` and
`ImportFailure`. It cannot redefine the meaning or completeness of source-scene selection,
retained/omitted object-kind counts, or scope-versus-loss classification. ADR 0036 owns the source
identities, stable digests, provenance, and loss representations carried by the record.

An import has zero or one finalized source-scope record. Zero means the selected format has not
declared a source-scope projection or selection did not complete; it never means an empty projection.
The builder is an incremental, ledger-bound capability borrowed from `ParseScope`; it checks and
charges count, detail-cell, string, digest-work, and live-capacity growth before copying caller data.
The mandatory outcome, exact typed retained/omitted counts, and stable digest are never collapsed.
Optional identity/detail retention has exactly two fidelity states:

```text
SourceScopeEvidenceFidelity =
    Exact
  | DigestCollapsed(ProvenanceEvidenceLossId)

ImportLossSubject =
    RawRange(SourceIdentity, ByteRange)
  | SourceScope(DocumentIdentity, SourceScopeDigest)
```

The names describe the required semantic shape, not a frozen public Rust layout. A source-scope
collapse uses the typed `SourceScope` subject and never fabricates a raw byte range. It is permitted
only when the frozen `LossPolicy` allows `ImportLossKind::ProvenanceEvidenceLoss`; otherwise budget
exhaustion latches a session-terminal scope cause. An authorized collapse retains exact counts and
digest, emits one bounded loss fact and diagnostic, and records that loss ID in the fidelity state.
It does not authorize ADR 0039 source-semantic loss.

Finalization prepares and reserves the record, loss fact, diagnostic, and all container growth before
publishing any of them. If validation, charging, reservation, allocation, loss-evidence retention, or
diagnostic preparation fails, no partial record is submitted and the first cause is latched. Final
submission is one-way: a duplicate, invalid, or uncharged record also latches a session-terminal scope
cause. Once submitted, the parser cannot replace it, and any later parse, construction, validation,
cancellation, or finalization failure retains the same record in the terminal `ImportFailure`. This
channel does not change the normative
`ParseResult<SceneDraft>` return shape or grant the parser access to an outcome constructor.

Dependency access has one result domain regardless of the selected role's runtime requirement. The
names below describe the required semantic shape, not the public Rust layout:

```text
dependency(origin, role, hostile_reference) -> ParseResult<DependencyOutcome>

DependencyOutcome =
    Available(DependencySnapshot)
  | Unavailable(OptionalDependencyEvidence)
```

For a `Required` role, every unavailable physical result becomes a latched `ScopeAbort`, so
`DependencyOutcome::Unavailable` is unreachable. For an `Optional` role, that variant is returned
only after the frozen role table and caller loss policy authorize omission. A single return type lets
the runtime role table own requiredness without making method selection or a generic type parameter a
second requiredness input.

The resolver may return `Unavailable` only when the selected format contract classifies that
dependency role as optional and the caller's loss policy permits the resulting omission. The scope
records one bounded diagnostic/loss fact and preserves the normalized identity, original lower-layer
cause owner, cached-failure disposition, and ADR 0010's atomically frozen negative observation
(`proof | none`) in `OptionalDependencyEvidence`. `Unavailable` is a
successful resolver outcome, not a `ParseFailure`; it does not latch the session and the importer may
continue building geometry. A denied, missing, failed-open, or over-budget optional acquisition still
consumes its non-refundable accounting under ADR 0022. Acquisition-byte/work exhaustion is marked
session-dependent and makes ADR 0041 input replay status `ObservedOnly`. Cycle, depth, node, edge,
graph-work, proof, manifest, finalization, cancellation, and deadline failures terminate regardless
of optionality. Once a snapshot is `Available`, format-owned
content decoding and recovery follow the separate diagnostic/loss contract; malformed content is not
reclassified as a resolver miss.

Requiredness tokens, outcome/evidence constructors, and loss disposition are private to the session
resolver. Format code may submit only a dependency role admitted by its frozen descriptor contract;
it cannot forge `Unavailable`, relabel a required role, or attach a fabricated lower-layer cause.

After every logical reference has independently passed role and policy authorization, the dependency
table caches the first immutable snapshot or identity-terminal failure by ADR 0010's
authority-qualified key `(authority domain, normalized resource identity)`. Here
*identity-terminal* means that later authorized references to that same key observe the same snapshot
or failure without another physical open; it does not by itself make the import session terminal.
References in another scope or resolver namespace never share the entry, and a denied alias cannot
consult it. Requiredness and loss disposition remain per-reference decisions outside the physical
key. A required request for a cached failure, an optional omission rejected by format or loss policy,
cancellation/deadline, or any capability failure for which safe continuation is forbidden is instead
returned as a session-terminal `ScopeAbort` and latched before control returns to the importer.
Required and optional requests therefore share one physical result only inside one authority domain
without conflating cache stability with outcome finalization.

Rust has no friend-crate visibility. A format crate can call the same public core validator as any
downstream crate, but `FormatImporter::parse` has no `ValidatedScene` return path and the session never
accepts parser-supplied proof. Any such internal proof is irrelevant to import authority: the parser
must still return a draft, and the session validates that draft again before outcome finalization.

Work, reservation, cancellation, deadline, and progress operations are borrowed capability views over
`baozi-operation`; they do not expose the coordinator or ledger state machine itself. This lets core
construction and validation consume the same low-level capabilities without depending upward on
`baozi-import`.

An in-process importer, IO backend, resolver, option codec, or callback is trusted extension code.
Baozi catches its panic only when the platform can unwind and converts it to a typed boundary failure.
This does not contain abort, process exit, infinite loop, arbitrary allocation, opaque dependency
copies, or hostile IO. A declared opaque-backend allowance is admission/accounting evidence, not proof
of actual runtime memory use. Hosts must use an admitted process-isolation boundary with an OS-enforced
limit for hostile plugins or any opaque decode path that claims hard memory containment.

### Resource-Graph Evidence Transport

This ADR owns the concrete session capability, staging, and success/failure transport used for
resource-graph evidence. ADR 0041 owns origin semantics, graph invariants, structural completeness,
unavailable replay classification, and canonical manifest content. Adding
that evidence does not give format code access to the graph builder or move graph meaning into this
lifecycle ADR.

The session creates exactly one private resource-graph builder with the bound primary root. A
`DependencySnapshot` returned by `ParseScope` carries only the opaque origin capability needed for
references authored in that snapshot. Format code can pass such a capability back with a stable role
and hostile reference; `ParseScope` derives requiredness only from that role's sealed frozen
descriptor contract. Format code cannot inspect, serialize, forge, retain across
sessions, compare/hash/order/render it for semantic branching, or substitute a caller-created path
for that capability.

Terminal-edge evidence capacity is prepared before acquisition. If the session cannot retain and
canonicalize one complete terminal edge, it latches `ScopeAbort` even when the format role is
optional. Once that envelope exists, a typed acquisition outcome may continue as optional only under
the descriptor/loss rules below and ADR 0041. This transport rule prevents aggregate edge,
manifest-byte, or finalization-work exhaustion from being mistaken for an ordinary optional miss.

The prepared envelope has a sealed slot for ADR 0010's first physical result and its inseparable
negative observation. Publication stages `Snapshot` or `Failure(..., Proof | None)` exactly as
returned by the authority-qualified cache; it cannot ask the resolver again, append proof after a
cache hit, upgrade `None`, or rewrite namespace/target/class/code. ADR 0041 canonicalizes the staged
inert proof and derives input replay status only after the complete graph is ready.

Successful finalization atomically publishes exactly one structurally complete
`SuccessfulInputManifest`. The Scene, manifest, diagnostics/losses, source-scope evidence, and ledger
evidence are prepared as one publication bundle; no field becomes visible if manifest
finalization, validation, charging, allocation, cancellation, or publication preparation fails. Once
the bundle is prepared, its ownership handoff is infallible.

Capability-time graph failures reached through `ParseScope` latch its opaque `ScopeAbort`. DAG
validation, quotienting, canonicalization, allocation, ledger finalization, or publication-preparation failures
detected after parser return are import-owned finalization causes created directly by `ImportSession`;
they do not masquerade as parser-produced `ParseFailure` values.

Failure finalization retains only bounded terminal partial graph evidence accumulated before the
first cause. It never constructs a `SuccessfulInputManifest` or complete-input claim. The session
pre-admits one minimal terminal envelope before risky work; if rich failure-outcome preparation itself
fails, all partial rich evidence is destroyed and that envelope records the preparation cause without
another fallible allocation. Unfinished graph states are discarded before the immutable failure is
published. Both success and failure evidence are Baozi-owned inert values and retain no resolver
callback, authority token, native handle, cache entry, backend object, or foreign destructor.

### Parse Failure Transport

The object-safe parser signature is frozen as:

```rust
pub type ParseResult<T> = Result<T, ParseFailure>;
```

`baozi-import` owns `ParseFailure`. Its representation and classification constructors are private, so
extension code cannot forge an IO, budget, cancellation, validation, or format category. It has two
session-internal branches with distinct cause ownership:

1. A session-terminal `ScopeAbort` is produced only by `ParseScope` and the IO, dependency, budget,
   control, diagnostic, Scene-authoring, or core capability methods it exposes after the capability has
   determined that the session cannot safely continue. Those methods return `ParseResult<T>` directly,
   making `?` the normal propagation path. The abort retains the original lower-layer cause and owner;
   an IO error remains IO-owned, cancellation remains operation-owned, and a construction/core error remains
   core-owned rather than becoming a format parse error. A permitted optional-dependency `Unavailable`
   outcome is deliberately outside this branch.
2. `FormatFailureEnvelope` seals the selected descriptor identity, a validated stable code, and
   bounded packed structured details into a Baozi-owned inert `FormatFailureEvidence` value. The
   selected `ParseScope` reserves the packed-detail storage, takes a format evidence producer by
   value, validates and copies its code, and invokes it synchronously through a bounded sink. It
   never retains that producer, its concrete format error, a backend/library error, or another object
   with foreign methods or destruction behavior. No generic `From<dyn Error>` or arbitrary category
   constructor exists.

A failure returned by the session's final central-validator call is not parser transport and does not
enter `ParseFailure`; it remains a core-owned validation cause finalized directly by `ImportSession`.
The `ScopeAbort` core branch covers construction-time core authoring-capability failures reached
while the parser is still running.

A session-terminal capability-produced abort is also latched before it is returned. A trusted parser
may inspect or accidentally catch the opaque error, but discarding it and returning `Ok(SceneDraft)`
cannot turn the operation into success; the finalizer observes the first terminal latch. This makes
`?` ergonomic without relying on every extension to preserve the classification manually. In
contrast, an importer continues after `DependencyOutcome::Unavailable` through an explicit
success branch, never by catching or suppressing `ParseFailure`.

The producer writes only stable enums, bounded scalars, IDs, ranges, and admitted strings through the
scope-owned sink rather than returning an input-sized preformatted `String`. Codes, detail keys, and
string values may be computed dynamically; the host copies their bytes into one admitted packed
arena. Packing is fallible and bounded by the import diagnostic/detail budget. Code, detail, and
producer-destructor callbacks run behind the same non-reentrant panic boundary. Before the constructor
returns, the host destroys the producer and every native error it owns; no borrowed data or foreign
object remains.

`FormatFailureEvidence` contains only Baozi-owned declarative data and may implement `Error` so
`Error::source(&FormatFailureEnvelope)` returns that one inert host-defined cause. The envelope offers
typed stable code/detail inspection, but no concrete-source downcast and no backend/library source
chain. It contains no `Box<dyn Error>`, extension trait object, callback, function pointer, native
handle, or plugin-owned destructor. `Debug`, `Display`, serialization, and reporting all derive from
the same sanitized descriptor/code/details representation.

The import finalizer consumes `ParseFailure` and preserves its branch, cause owner, accumulated phase,
diagnostics, and ledger evidence in `ImportFailure`. The facade composes that failure rather than
copying it into another format-error enum. Option-codec failures remain pre-session `RequestError`
values only for submitted request layers; registry-default failures are `RegistryBuildError` under ADR
0016. Neither enters `ParseFailure`.

### Prepared Outcome Publication

The session finalizer has a fallible preparation boundary and an infallible ownership-handoff
boundary. Success preparation consumes the session-owned construction with the exact draft
descriptor, validates the canonical aggregate, completes the successful input manifest,
diagnostics/losses, source-scope evidence, ledger snapshot, account brand, physical leases,
private canonical-attribution and outcome-evidence-attribution destinations. It reserves
`ReservedSceneGeneration` as the final fallible pre-freeze identity acquisition; it does
not freeze or construct an earlier `ValidatedScene`. Reservation rejection or exhaustion leaves the
generation domain unchanged, destroys unpublished success storage, appends the reservation cause,
and selects ordinary rich-failure preparation with zero canonical output.
After successful reservation, any later freeze/product-graph preparation failure, cancellation, or
caught unwind burns the private gap. A non-builder unwind may select rich-failure preparation after
candidate/publication cleanup is verified and `StartupBaselineOwner` remains exact; a retained
candidate/report builder panic selects containment instead. Failure preparation validates the
cause, last phase, bounded partial evidence, ledger snapshot, account brand, physical leases, zero
canonical attribution, and private outcome-evidence attribution.

Preparation performs every allocation, validation, checked arithmetic operation, ledger/account
admission, reservation transition, cancellation poll, user/extension callback, and final control
downgrade. The operation layer returns a closed, already classified terminal transport containing
exactly one `PreparedSuccess`, `PreparedRichFailure`, or `PreparedMinimalFallback`; the import layer
wraps that exact variant without erasing its typestate. There is no opaque success-capable prepared
outcome whose branch is decided during handoff.

Preparation returns an exact wrapper only after it applies destination-qualified canonical and
outcome-evidence commitments and current operation-live attribution is zero. No pending or prepared-
attribution token survives. Only `PreparedSuccess` structurally owns the still-domain-unexposed
canonical owner and private generation reservation. `PreparedRichFailure` and
`PreparedMinimalFallback` structurally contain neither, while all exact wrappers may own the already
qualified but domain-unexposed evidence owner. Each exact handoff is a pure consuming split: success
converts the reserved nonce into `SceneGenerationId`, constructs `ValidatedScene`, and exposes the
canonical/evidence owners as `CommittedCanonicalOutput` and `PublishedOutcomeEvidence`; failure
exposes evidence only. Handoff performs no generation allocation, attribution reclassification,
checked counter mutation, terminal classification, fallible lock, or extension call and cannot
return an error or panic under the published contract. Dropping prepared success burns an invisible
never-reused nonce gap. Ordinary drop, and a caught non-builder unwind only after its cleanup
completes, destroys domain-unexposed publication owners in storage-before-account order. It does not
reopen the ledger, roll back destination commitments, or expose a public terminal.

Final publication eligibility is still part of preparation. It checks the shared account poison
latch under ADR 0044's linearization state before any exact wrapper is returned. Poison-first,
including a poison that follows an earlier successful start/retain admission, returns
`FinalizationContainmentError`, exposes the shared account poison snapshot/count/evidence, publishes
no prepared terminal, and makes handoff unreachable. If publication eligibility linearizes first,
the exact wrapper is already authorized and its pure handoff performs no new account check.

The session creates one assembly-independent minimal terminal envelope before primary IO. It is
already allocated, bound to the session's physical account, and admitted for one fixed-size failure
cause plus complete dynamic ledger-dimension/phase evidence. If appending, validating, or preparing
rich failure evidence after downgrade itself fails, or an invalid root or equivalent non-unwind
integrity fault makes rich assembly unsafe, the finalizer destroys all partial rich evidence, fills
that envelope without allocation, and prepares it through prevalidated slots. After successful
startup, a non-builder unwind may use the same envelope only after candidate/publication cleanup is
verified, the exact `StartupBaselineOwner` remains intact, dependency/final-owner cleanup completes,
and operation-live attribution is zero. A retained candidate/report builder panic never enters this path.
Generation reservation rejection/exhaustion alone does not enter this path. There is no recursive
attempt to allocate another `ImportFailure`.

Canonical and outcome-evidence destination commitments are independent preparation results. A
success wrapper privately owns both already qualified owners until handoff exposes
`CommittedCanonicalOutput` and `PublishedOutcomeEvidence`. An ordinary rich-failure wrapper owns and
exposes only outcome evidence; this includes generation reservation rejection/exhaustion. An invalid root, failure to append/validate/
prepare rich failure, or equivalent non-unwind integrity fault with verified candidate cleanup and an
intact `StartupBaselineOwner` uses
the minimal fallback and also publishes evidence only. All three exact forms reach zero operation-live attribution during
preparation, before their pure handoffs, while physical leases remain bound to returned storage.

ADR 0044 owns all startup and later retained-storage cleanup facts, evidence precedence, and
membership-specific release rules. The import finalizer consumes only the resulting typed lifecycle
outcome and cannot reinterpret physical containment as a clean report-quality downgrade.

If the process survives but any dependency extraction, destructor completion, backing-storage
deallocation, lease release, or attribution release cannot be verified, the finalizer must not claim
one of those three exact prepared forms. It keeps the uncertain payload, physical lease, and operation
attribution coupled, marks the affected operation/account/extension path poisoned, and returns a
fixed Baozi-owned `FinalizationContainmentError`. The physical account latch/count/fixed first
evidence is updated atomically under its counter lock. That error carries only a stable reason and
read-only account-poison snapshot; it publishes no `ImportFailure`, minimal fallback, operation-ledger snapshot, or zero-leak
claim, and it performs no optimistic refund. Double panic, `panic = "abort"`, process exit, and OOM
have no in-process terminal guarantee and require process isolation.

### Success Outcome

`ImportReport` is immutable and contains at least:

- `ValidatedScene`, never an unvalidated mutable Scene;
- selected immutable format descriptor;
- complete sorted detection evidence;
- bounded diagnostics and explicit loss evidence;
- an immutable attempted, consumed, peak-live, committed-canonical-output,
  published-outcome-evidence, and final-current ledger snapshot;
- sealed import phase/source/provenance evidence;
- one structurally complete successful input manifest with typed replay status.

No production report field exposes the Proposed ADR 0043 producer, closure-token, or import-build-
identity types.

The report does not contain a setter, `map_scene`, `map_scene_with`, arbitrary stage transition, or a
parser-supplied stats object. Structural edits consume the validation proof through the core edit
contract and yield a new proof outside this import lifecycle.

Successful preparation releases all scratch/opaque reservations, applies independent canonical and
outcome-evidence destination commitments, owns one reserved generation, and reaches zero session-
owned live attribution. The subsequent pure handoff consumes that reservation into the validated
Scene and exposes/moves the already qualified report owners without generation allocation,
attribution transition, or counter mutation. The ledger's peak still includes
output while it overlapped with snapshot, backend, normalized, validation, or report storage.

### Failure Outcome

`ImportFailure` contains:

- a typed cause category and owning lower-layer cause;
- last completed import phase;
- normalized source identity and selected format when known;
- complete detection evidence accumulated before the failure;
- bounded diagnostics/loss evidence;
- bounded terminal partial resource-graph evidence without a successful-manifest claim; and
- immutable ledger evidence including attempted and consumed counters, peak-live, zero committed
  canonical output, typed nonzero-or-zero published outcome evidence, and zero final session-owned
  live usage after cleanup.

Unsupported format, detection failure, dependency failure, parse failure, validation failure,
resource exhaustion, cancellation/deadline, and trusted-extension panic remain distinguishable.
Formatting a failure cannot discard its structured cause. If rich failure-outcome preparation failed,
the cause identifies that preparation failure and only the pre-admitted minimal evidence is present;
this is still a successfully published `ImportFailure`, not canonical output and not a second
publication failure.

Failure preparation applies only the outcome-evidence destination commitment, proves zero operation-
live attribution, and reserves or publishes no Scene generation. Its exact wrapper privately owns the
already qualified but domain-unexposed evidence. Handoff only exposes that owner as
`PublishedOutcomeEvidence`; it performs no canonical or attribution transition.

### Post-Process Boundary

Raw import success means source-oriented canonical IR has passed central validation. Post-processing is
an explicit subsequent operation or explicitly composed facade request and returns its own report.
This ADR does not define post-process internal stages, transactions, remaps, or derived-data semantics.
A composed facade preserves the import report/evidence and post-process report/failure separately; it
does not mutate an `ImportReport.stage` field.

### Crate Error and Capability Ownership

| Crate | Owns | May expose upward | Must not own/construct |
| --- | --- | --- | --- |
| `baozi-operation` | Runtime-neutral control/coordinator, reservation/ledger state machine, physical account, sole private generic final-owner/retained foundation, operation-owned concrete metered retained store, and frozen prepared-publication mechanism | Concrete sealed `ScalarSequenceBlock`, `BytesBlock`, `Utf8Block`, and bounded `RecordBlock` construction; qualified non-owning slots; `PhysicalStorageAccount`; owner-neutral ledger evidence. Raw generic payload, pending charge, reservation, and lease controls remain private | Public/domain generic retained authority, arbitrary owning payload/backing storage, caller-declared footprint, Scene, IO, request policy, import/post-process/core phases, format causes, domain outcomes, or a second retained/final-owner implementation |
| `baozi-core` | Opaque single-use `SceneConstruction`, bounded authoring surface, descriptor-only `SceneDraft`, sealed unstable canonical in-memory schema, non-owning `CanonicalBlockRef<Schema>` projections and semantic views, central validation, validation proof, and domain validation/edit errors | Typed core causes and `ValidatedScene`; standalone callers may drive explicit-account or private-account construction entry points but cannot separate a draft descriptor from its owner | Public owning allocation wrapper, stable internal wire, physical protocol or `Drop` duplication, IO, import phase/provenance, registry, parser policy, or facade-composed error |
| `baozi-io` | Asset identity/path, bound handles/snapshots, backend IO errors | Typed IO causes and capability-bound resources | Optionality/loss, Scene, import outcome |
| `baozi-import` | Request config/policy, session, ownership and orchestration of one core construction, import phase vocabularies, Probe, DetectionScope, ParseScope, resolver, effective budgets, diagnostics, private detection/parse failure classification, and detection/import outcomes | Typed request/import failure and report evidence; a borrowed capability-limited authoring surface to the selected importer | Validation-proof construction, direct canonical retained ownership, shared operation mechanism duplication, format-private model, post-process stage, public facade error duplication |
| `baozi-format-*` | Private decode/normalized model and bounded Baozi format causes/codes | `DetectionResult<DetectionOutcome>` and `ParseResult<SceneDraft>`; synchronous packing of stable failure facts through the selected ParseScope | Raw IO, parser-supplied proof/outcome, arbitrary detection/parse failure classification, phase/provenance construction, or retaining backend/plugin objects in public outcomes |
| `baozi-postprocess` | Post-process phase vocabularies, effective operation budget, compiled plans, step transactions, remaps/losses, report/cause | Distinct typed post-process result using `baozi-operation` mechanisms | Dependency on `baozi-import`, import phase/outcome, shared operation mechanism duplication, or format-private state |
| `baozi` facade | Public request/error composition and long-lived bounded runtime/profile ownership | Stable ergonomic API preserving lower structured causes; private per-import account creation through its retained factory; advanced explicit-account entry without exposing ledger/physical types in the common flow | Global/per-request/unbounded factory, canonical inheritance authority, duplicate domain state, or bypass constructors |

Each lower crate owns its domain cause. The facade composes, rather than stringifies or redefines, those
causes for public use.

## Alternatives Considered

### Option A: Let each importer return `ImportReport`

This is locally convenient but lets parsers forge stats, validation, phase, and selection evidence.
Rejected.

### Option B: Keep a public mutable `ImportContext` and assignable stage enum

This requires convention to prevent bypasses and allows helpers to publish invalid transitions.
Rejected.

### Option C: Session-owned lifecycle with capability-limited ParseScope and one finalizer

This centralizes every outcome path and makes parser authority explicit and testable. Chosen.

## Consequences

Positive:

- Open, detection, parse, validation, cancellation, and panic failures share one evidence-rich outcome.
- Parsers cannot bypass dependency policy, budgets, diagnostics, or central validation.
- Validation proof can be obtained only from the core validator and cannot be forged by field/enum
  construction; import lifecycle/provenance evidence remains session-only.
- Post-processing remains explicit and does not rewrite import history.

Negative:

- Existing parser signatures and facade helpers must break in one cutover.
- Capability plumbing is more explicit than a monolithic context.
- Trusted plugin isolation still requires a host process boundary.

## Risks and Mitigations

| Risk | Severity | Mitigation |
| --- | --- | --- |
| A convenience helper bypasses the session | Critical | Keep constructors private and test every public facade path with instrumented IO/invalid drafts. |
| Session bootstrap fails before the terminal envelope exists | Critical | checked complete sealed-cell/header semantic layout plus per-backing `CapacityPlan`, ledger-first/physical-second aggregate admission, structural rollback only for typed no-mutation second-plane rejection, fail-closed coupled containment for panic/post-mutation poison/uncertain disposition, fixed-size allocation-free `SessionStartError`, no asset IO, and exactly-one returned-terminal guarantee begins only after `try_start` succeeds |
| Two accounting locks are mistaken for one cross-plane transaction lock | Critical | each plane admits its complete aggregate under its own non-nested lock; a private guard makes only external session acceptance atomic and tests prove no shared internal linearization point is claimed |
| Process-local evidence keys outlive their mapping or enter stable identity | Critical | the Baozi-owned result/evidence owner retains the `StoreKeyDomain` anchor and immutable normalized catalog through query lifetime; plugins receive only host-minted keys, and transcript/digest/wire tests exclude the runtime brand |
| A cloned explicit account bypasses containment poison | Critical | operation start uses ADR 0044's shared monotonic account latch; every clone rejects with typed `AccountPoisoned`, and import/facade APIs expose no reset, ignore, state replacement, or reuse path |
| Account poison races a previously admitted import | Critical | start/retain charge remains coupled, all later admissions reject, and final publication eligibility observes poison under the account linearization state and returns fatal containment before an exact prepared wrapper exists |
| ParseScope grows back into a raw service locator | High | Capability allowlist, private fields, and compile-fail boundary tests. |
| Failure conversion drops lower-layer cause/evidence | High | Exhaustive outcome tests at every lifecycle phase. |
| Public core validator is mistaken for import authority | High | The validator is the sole proof constructor, but accepts/returns no import phase or provenance; the session always validates the returned draft itself. |
| Detector catches a scope abort and returns a match | Critical | `DetectionScope` latches the first terminal cause; zero/repeated/oversized work, polling during work, and unfinished permits are terminal even when the opaque abort is discarded. |
| Parser catches a session-terminal capability abort and returns success | Critical | Capability methods latch the first opaque `ScopeAbort`; finalization fails even if trusted parser code discards the returned value. Permitted optional unavailability uses an explicit non-abort outcome. |
| Optional evidence exhaustion is mislabeled as an unavailable dependency | Critical | Pre-admit the complete terminal edge/finalization envelope; failure to preserve it always latches `ScopeAbort`. |
| Partial graph state is published as a successful manifest | Critical | Only the private finalizer can construct `SuccessfulInputManifest`; failure transport has a distinct bounded partial-evidence type. |
| Failure-outcome preparation recursively needs another allocation | Critical | Pre-admit one checked-layout minimal account-bound terminal envelope before risky work; destroy partial rich evidence and perform one infallible prepared handoff only after cleanup is proved. |
| Prepared outcome handoff still reaches a failure or panic boundary | Critical | Preparation completes final downgrade and destination commitments, returns an exact typed terminal wrapper with zero live attribution, and performs all allocation, validation, accounting, control, callback, and account work; type-specific handoff only exposes owners and cannot fail or panic. |
| A retained-builder panic is reported as fallback | Critical | Startup panic returns `SessionStartError` with side containment; later candidate/report panic returns fatal containment; fallback requires verified candidate cleanup and intact `StartupBaselineOwner`. |
| Import lifecycle reinterprets retained containment as a report downgrade | Critical | Consume ADR 0044's typed startup/finalization result without reproducing or weakening its physical cleanup protocol. |
| Generation allocation remains after attribution freeze | Critical | Reserve `ReservedSceneGeneration` as the final fallible pre-freeze identity acquisition; rejection/exhaustion consumes no nonce and becomes ordinary rich failure, any later preparation failure/cancellation/caught-unwind/prepared-success drop may burn the gap, and only exact `PreparedSuccess` can own/consume a reservation. |
| Failure evidence is counted as canonical output or loses its physical owner | Critical | `PublishedOutcomeEvidence` is independent of zero committed canonical output and every retained slot keeps its account-bound lease. |
| Import code duplicates or bypasses retained storage authority | Critical | `baozi-operation` keeps the generic rail private and exposes one concrete store; core owns only sealed schema plus non-owning projections, and import owns only sealed outcome composition. |
| Import and postprocess authorities overlap | Medium | Import ends at validated raw report; postprocess owns a distinct operation/report. |
| Common facade hides an unbounded/global/per-request factory | Critical | A long-lived runtime owns one finite documented factory and private per-import accounts; advanced explicit-account paths remain deliberate and do not grant lineage. |

## Success Metrics

| Metric | Target | Measurement |
| --- | --- | --- |
| Single visible result | after successful bootstrap, exactly one `ImportReport`, `ImportFailure`, or `FinalizationContainmentError`, with no partial field visibility; the containment error is not a prepared fallback | fault injection at every lifecycle, cleanup, and preparation boundary |
| Destination independence | preparation applies canonical/outcome commitments independently and exact wrappers privately own the still-domain-unexposed values; exact handoff alone exposes committed canonical output and `PublishedOutcomeEvidence` | three-path publication snapshot matrix |
| Failure fallback | failure to append/validate/prepare downgraded rich evidence or invalid root/equivalent non-unwind integrity fault returns the assembly-independent envelope only after verified candidate cleanup with intact `StartupBaselineOwner`; generation rejection/exhaustion remains rich failure, while retained candidate/report builder panic or uncertain cleanup returns fatal containment | candidate-cleanup, allocator/deallocation counters, poison state, and terminal-cause assertions |
| Startup lifecycle mapping | every failed retained startup yields `SessionStartError` and no session/terminal; successful startup yields the baseline required for later downgrade; later retained-builder containment yields `FinalizationContainmentError` | operation integration, session acceptance, and no-terminal probes |
| Poison fail-closed | poison-first explicit-account start has zero ledger/work/IO/refcount/allocator effects; start/retain-first then poison preserves charge but cannot prepare or hand off any import terminal | deterministic two-order barriers, clone/account snapshots, finalization probes, and explicit-account reuse rejection |
| Infallible handoff | only exact prepared success owns a reserved generation; every exact wrapper has zero live attribution and zero allocator, checked generation/account transition, classification, panic source, or extension callback in handoff | exhaustion, API-shape, prepared-drop, and injected-failure/panic probes |
| Owner uniqueness | zero import-owned physical charge/lease/canonical allocation implementations and zero public generic retained authority | crate graph, rustdoc, compile-fail, and production source scans |
| Facade boundedness | common imports use one runtime-held bounded factory and private account per import without exposing physical/ledger types | default/custom profile, repeated import, lifetime, and packaged API tests |

## Acceptance Criteria

| Contract | Testable acceptance |
| --- | --- |
| Session-before-IO | Invalid requests create no session and perform zero IO; a primary-open failure observes one created session and one finalizer. |
| Session bootstrap | Invalid/zero/exact account-factory and physical limits plus shared account-poison start admission, key normalization, count conversion, complete `FallbackDimensionCell<D>`/`FallbackPhaseCell<P>` `Layout::array`, ordered semantic `Layout::extend`/`pad_to_align`, per-actual-allocation `CapacityPlan`, and aggregate requested-envelope/block checked sum either return one allocation-free `SessionStartError` with zero accepted session/IO or produce a private ready pair. Tests prove one complete ledger aggregate admission, then one complete physical aggregate admission, no nested locks or shared cross-plane linearization point, deterministic first-plane rollback only on typed no-mutation second-plane rejection, coupled fail-closed containment on panic/post-mutation poison/uncertain disposition, and zero retained allocator entry until both succeed. A poison-first typed account rejection leaves no accepted ledger/work/IO/refcount/allocator state. No dimension-only, unchecked `size_of`, or aggregate-layout-as-single-backing formula is accepted. |
| Startup retained outcome | Operation integration fixtures prove every retained-start failure returns one `SessionStartError` with no accepted session or terminal, while successful startup supplies the exact baseline required for later downgrade. Retained builder containment after startup returns `FinalizationContainmentError`; physical cleanup conformance remains ADR 0044's test authority. |
| Fallback evidence ownership | Compile-fail, shape, and plugin-unload tests prove each cell contains one inert Baozi-owned copied key plus its snapshot; keys have no `Drop`, external/plugin object, trait object/vtable, or function pointer and are normalized before the first ledger entry. The Baozi-owned result/evidence owner keeps the process-local `StoreKeyDomain` anchor and immutable normalized mapping through query lifetime; plugins receive only host-minted keys. Brand variation leaves semantic transcripts, digests, wire values, cache/build identities, and user-visible meaning unchanged. `FallbackHeader<P>` retains schema revision, terminal cause, last completed phase, counts, operation-wide current/peak live bytes, active reservations, and current/peak evidence-storage bytes; no global field is reconstructed from cells. Prepared minimal fallback has zero current live, active reservations, and current evidence storage, preserves both peaks, and remains queryable without plugin code. |
| Pre-session panic | Submitted-layer codec panic is one bounded `RequestError`; default-codec panic prevents snapshot creation as `RegistryBuildError`; both have zero sessions, finalizers, import ledgers, diagnostic streams, and asset opens. |
| Single finalization | After session creation, success, every failure phase, cancellation, and caught unwind call the outcome finalizer exactly once; it returns one report/failure when cleanup is proved or one fatal containment error when release remains uncertain. Abort/double-panic/exit/OOM are asserted only by the isolating parent process. |
| Success report atomicity | Injected failures at every success-preparation allocation, validation, generation reservation, destination-commit/accounting, product-graph, account-brand/poison eligibility, cancellation, and transition point publish no partial field or generation; rejection/exhaustion consumes no nonce and downgrades to rich failure, later failure may burn a successful reservation, poison-first eligibility returns fatal containment with no exact wrapper, and exact `PreparedSuccess` applies canonical/evidence commitments, reaches zero operation live, privately owns domain-unexposed values, then permits only pure consuming handoff. |
| Account poison race | Barrier tests force poison-first and start/retain-first orderings across multiple explicit-account clones. Poison-first returns `SessionStartError::AccountPoisoned` with zero work/counter/IO/refcount/allocator deltas. Start/retain-first keeps the admitted charge, rejects every later retain/share, records the same account poison/count/evidence in all snapshots, and returns `FinalizationContainmentError` during final preparation with no report/failure prepared handoff. Source/API tests find no reset/unpoison/reuse/ignore route. |
| Failure outcome atomicity | Ordinary failures, including generation rejection/exhaustion, prepare zero canonical commitment, one exact bounded outcome-evidence commitment, and zero operation live before one pure handoff exposes the domain-unexposed owner as `PublishedOutcomeEvidence`; their wrappers structurally contain no reservation. |
| Failure-publication-failure atomicity | Injected failure to append/validate/prepare rich-failure evidence or invalid root/equivalent non-unwind integrity fault publishes only the assembly-independent pre-admitted minimal terminal envelope after verified candidate cleanup with intact `StartupBaselineOwner`, zero committed canonical output, zero operation live, no public generation, and no recursive allocation. A generation-reservation fault remains ordinary rich failure. Retained candidate/report builder panic, ownership mismatch, or uncertain release keeps the exact affected owners/charges/attributions coupled, poisons the path, and returns `FinalizationContainmentError` without fallback/refund/zero-leak claim. |
| Prepared handoff API | API-shape and instrumentation prove safely completed preparation returns exact `PreparedSuccess`, `PreparedRichFailure`, or `PreparedMinimalFallback`, not one success-capable opaque wrapper. No exact wrapper owns pending/live or prepared-attribution tokens; only exact success may own one private generation reservation. Their type-specific consuming splits cannot allocate a generation, classify terminal state, validate, poll control, invoke a callback, reclassify attribution, mutate a counter/account, return `Result`, or panic. Dropping exact success burns its gap; dropping any wrapper destroys domain-unexposed publication owners in storage-before-account order without reopening the ledger or exposing a terminal. `FinalizationContainmentError` is a distinct pre-prepared fatal result. |
| Capability boundary | Compile-fail/API-shape tests prove detectors and parsers cannot reach raw IO, other options, reports, phases, arbitrary detection/parse failure constructors, dependency-requiredness relabeling, or a parser-supplied proof return path. |
| DetectionResult shape | A heterogeneous `dyn FormatImporter` receives `Probe`, `ReadHint`, and `DetectionScope` and returns exactly `DetectionResult<DetectionOutcome>`; its opaque abort cannot be constructed, sent, or used to suppress the session latch. |
| Detector work permit | Detection permits are single-outstanding and bounded to 4 KiB; begin polls then charges attempted work, complete charges consumed then polls, and zero/repeated/poll-during/drop/forgotten-permit violations latch the first terminal cause. |
| Core proof authority | Public/downstream code can obtain `ValidatedScene` only by completing a matching core construction or validated edit transaction, while detached-draft validation and direct proof construction/forgery fail to compile. |
| Core storage authority | Import Scene construction reaches only operation's concrete sealed scalar/bytes/record store through core's non-owning `CanonicalBlockRef<Schema>` projections. Rustdoc/source/compile-fail gates find no public raw generic rail, arbitrary owning `T`, backing `Vec`/`Arc`/`Mutex`, caller-declared footprint, owning core wrapper, import/core physical charge, lease, rollback, final-owner, or second retained implementation. |
| Draft and generation boundary | `SceneDraft` contains only its construction brand and directory-slot descriptor. Missing, duplicated, already-consumed, foreign, caught-error, and panic-after-transition tests cannot publish success or recover reusable owner authority. Exact/exhausted domain tests prove reservation failure consumes no nonce, precedes freeze, and produces ordinary rich failure. Only exact-success handoff consumes a reservation into `SceneGenerationId`/`ValidatedScene`; any later preparation failure/cancellation/caught-unwind/exact-success drop may burn but never publish or reuse a gap, and failure wrappers structurally contain no reservation. |
| Facade account lifecycle | Common facade construction requires a finite documented profile, retains one factory across repeated imports, creates a fresh unpoisoned private account per import, exposes no physical/ledger types, and has no global/per-request factory; advanced explicit accounts do not permit independent-Scene inheritance and cannot start another import after poison. |
| Internal-layout exclusion | Reordering concrete blocks or changing slots, capacities, padding, or equivalent sharing leaves semantic Scene digest and public views unchanged; no internal store representation is a stable wire. |
| Import construction authority | `ImportSession` retains exactly one session-bound construction while `ParseScope` lends only its authoring surface. `parse` can return only that construction's descriptor; a parser cannot move/finalize the owner, submit proof, substitute another construction, suppress a latched core cause, or publish an invalid/foreign draft. The session consumes the exact descriptor, completes central validation and manifest ordering, and composes canonical/report publication without first exposing an independent `ValidatedScene`. |
| ParseResult shape | A heterogeneous `dyn FormatImporter` returns exactly `ParseResult<SceneDraft>`; capability methods compose with `?` without erasing the lower cause owner. |
| Dependency API shape | ParseScope exposes one `(origin, role, hostile reference) -> ParseResult<DependencyOutcome>` capability; a required role can return only `Available` or a latched error, while only an optional role plus loss authority can return `Unavailable`. |
| Optional dependency continuation | Missing, denied, failed-open, and acquisition-budget-exhausted optional fixtures return `Unavailable` only after complete terminal evidence is admitted and sealed role optionality/loss policy permit; acquisition-budget outcomes are session-dependent and make replay status `ObservedOnly`. Cycle/depth/node/edge/graph-work/proof/manifest/finalization/cancellation/deadline failures terminate the session. Permitted outcomes retain identity/cause/accounting/proof-or-none evidence, emit bounded loss diagnostics, preserve geometry, and perform one physical attempt per authority-qualified identity. |
| Identity versus session terminality | Repeated authorized references in one authority domain reuse an identity-terminal cached failure without reopening; requesting that failure as required or rejecting its optional loss produces and latches a session-terminal `ScopeAbort`. |
| Cache authorization | Equal logical names in two scopes or resolver namespaces remain isolated; every alias and role is authorized before lookup, so an allowed cache entry cannot satisfy a later denied reference. |
| Scope abort latch | Required dependency, IO, budget, cancel, diagnostic, builder, and core session-terminal abort fixtures retain their owning cause/phase and force final failure even when a test importer catches the opaque `ParseFailure`. |
| Evidence preservation | Every post-detection failure retains identical selection evidence plus bounded diagnostics/stats. |
| Source-scope evidence | No-record, one-record, empty projection, exact-budget success, preserve-or-fail excess rejection, authorized detail collapse, loss-evidence/diagnostic-budget failure before submission, duplicate/invalid submission, success, and failure-after-submission fixtures prove one sealed record with exact typed counts/digest, typed fidelity/loss subject, atomic publication, and unchanged `ParseResult<SceneDraft>` shape. |
| Resource-graph capability | Root/dependency origins are opaque, non-comparable, and session-bound; forged, stale, foreign, unavailable, or caller-path origins latch failure before graph/cache/storage access, while nested references use only the origin returned with their snapshot. |
| Resource-graph outcome transport | Success atomically retains one complete manifest and replay status; proof/none is frozen with the first cache failure and cannot be post-attached or upgraded; every injected later failure retains only bounded partial graph evidence and cannot construct a successful-manifest claim. Post-parse finalization errors are direct import causes rather than synthetic parser failures. |
| Phase separation | Tests independently advance ledger/progress phases and prove `last_completed` changes only after the mapped atomic lifecycle boundary succeeds. |
| Format evidence transport | Heterogeneous format errors enter only through the selected-scope synchronous packing constructor; each envelope retains its selected descriptor and exactly one Baozi-owned `FormatFailureEvidence` containing the stable code and bounded packed details. `Error::source` returns that evidence, whose own source is `None`; no concrete-source downcast or backend/plugin object survives construction. |
| No legacy transition | Production/public symbol search finds no `map_scene*`, assignable import stage, or parser-owned report constructor. |
| Crate ownership | Dependency and public-API checks find no backend type or executable backend object in public outcomes/stable schemas, no `baozi-operation` upward dependency, no postprocess-to-import edge, and no reversed core dependency. |

## Amendment History

### 2026-08-01: Make ImportSession own the opaque core construction

- Affected headings: `Decision`, lifecycle, `ParseScope Capabilities`, parse failure transport,
  prepared outcome publication, crate ownership, and acceptance criteria.
- Previous rule: the parser returned a descriptor-only draft, but the Accepted text still implied a
  detached public validator and did not state who retained the matching construction owner or how its
  accounting and publication authority composed with the single import session.
- New rule: after selection, `ImportSession` creates and retains exactly one core
  `SceneConstruction` bound to the session's existing account, ledger, control, manifest ordering, and
  publication context. `ParseScope` lends only a bounded authoring surface. The parser returns the
  matching descriptor; the session consumes both, preserves any latched typed core cause, runs central
  validation, then freezes canonical and report evidence in one terminal. Missing, foreign,
  duplicated, caught-error, and transition-panic paths cannot publish or restore reusable authority.
- Compatibility impact: pre-release ParseScope, construction, validation, and import-finalization APIs
  may break; `FormatImporter::parse(&mut ParseScope) -> ParseResult<SceneDraft>` remains the registered
  parser shape.
- Required review: one-ledger/account composition, owner/descriptor API shape, parser catch/foreign-
  draft tests, manifest-before-generation ordering, joint canonical/report publication, fail-closed
  transition panic, and direct ADR coherence review.
- Rationale: import can have only one accounting and publication truth. Letting a parser or detached
  validator own the canonical continuation would recreate a second lifecycle outside the session.

### 2026-07-27: Follow retained physical-ownership authority supersession

- Affected: frontmatter `depends_on`, session startup, final publication, containment risks, metrics,
  and acceptance criteria.
- Previous: current import lifecycle text cited ADR 0042, which combined durable retained ownership
  invariants with U4-specific implementation and measurement details.
- New: ADR 0044 is the sole retained physical-ownership and lineage authority. Import lifecycle,
  outcome, and containment mappings are unchanged. Older amendment entries preserve the ADR
  identifier in effect when they were recorded and do not restore ADR 0042 authority.
- Impact: none; this is an atomic authority-navigation update.
- Review: reciprocal supersession, Accepted dependency graph, current-text reference scan, and direct
  ADR review.
- Rationale: import consumes the physical protocol's durable outcomes without making its temporary
  implementation specification part of the lifecycle contract.

### 2026-07-22: Seal format failures as host-owned inert evidence

- Affected: `Parse Failure Transport`, `Crate Error and Capability Ownership`, and `Acceptance
  Criteria`.
- Previous: the decision required synchronous stable packing, but its acceptance row still allowed
  an optional inert source and the implementation retained an extension-owned
  `FormatFailureSource` object behind the public envelope.
- New: the selected scope takes a synchronous producer by value, copies dynamic code/detail bytes
  into exactly one Baozi-owned `FormatFailureEvidence`, contains callback reentry and panic, destroys
  the producer, and exposes a fixed source chain:
  `FormatFailureEnvelope -> FormatFailureEvidence -> None`.
- Impact: pre-release format/facade error APIs break; built-in native error wrappers and concrete
  downcasts are deleted, while stable code/detail inspection remains available.
- Review: implementation commit `b1f7608`, focused import/format/facade/fuzz/WASM gates, and two
  independent Codex reviews with no remaining P0/P1/P2.
- Rationale: a terminal outcome cannot be inert or safe after plugin unload while any extension
  vtable, borrowed text, native error, or plugin-owned destructor remains reachable.

### 2026-07-20: Consume the ledger's prepared-owner publication seam

- Affected headings: state diagram, `Prepared Outcome Publication`, `Success Outcome`, `Failure
  Outcome`, `Risks and Mitigations`, `Success Metrics`, and `Acceptance Criteria`.
- Previous rule: the lifecycle repeated ADR 0022's superseded private
  `PreparedCanonicalAttribution`/`PreparedOutcomeEvidenceAttribution` model and described handoff as
  reclassifying attribution.
- New rule: ADR 0022 alone owns attribution commitment. Import preparation consumes that operation
  seam after applying canonical/outcome destination commitments and reaching zero operation live.
  Exact import wrappers privately own already destination-qualified but domain-unexposed publication
  owners. Handoff only consumes the wrapper, exposes/moves those owners, and constructs
  `SceneGenerationId`/`ValidatedScene` for exact success; it performs no attribution transition.
  Prepared drop destroys the unexposed owners in storage-before-account order without reopening the
  ledger or exposing a report/failure.
- Compatibility impact: pre-release internal typestate vocabulary changes; the report/failure shape
  and stable-wire policy do not.
- Required review: import-to-operation type mapping, preparation snapshots, exact terminal ownership,
  generation-gap drop, pure handoff probes, and source gates that reject duplicate attribution
  typestates in `baozi-import`.
- Rationale: import owns lifecycle and domain outcomes, while operation owns ledger commitment. A
  second import-defined attribution state would duplicate authority and make infallible handoff
  misleading.

### 2026-07-20: Separate accounting-plane admission and retain evidence-key authority

- Affected headings: `Lifecycle`, `Risks and Mitigations`, and `Acceptance Criteria`.
- Previous rule: bootstrap said the ledger and physical account were admitted atomically without
  defining lock order or rollback, which could be read as one nested cross-plane lock or common
  linearization point. It also required copied inert keys but did not structurally keep their
  process-local mapping alive with every queryable result.
- New rule: bootstrap preflights the complete aggregate, admits it first in the operation-attribution
  ledger under that ledger's one lock, releases the lock, and admits the complete physical aggregate
  second under the account's one lock. A private two-phase guard rolls back the first graph only when
  the second admission returns a typed no-mutation rejection. Panic, poison observed after mutation,
  or uncertain physical disposition instead preserves coupled attribution/physical ownership and
  fails closed without refund. No path exposes an accepted session until both succeed; the first retained
  allocator is unreachable before the ready pair exists. This gives atomic external session
  acceptance without nested locks or a shared internal linearization point. The Baozi-owned
  result/evidence owner retains the `StoreKeyDomain` lifetime anchor and immutable normalized catalog
  until query lifetime ends. Plugins receive only host-minted keys, and runtime brand values are
  excluded from stable semantics, digests, transcripts, wire, cache, and build identity.
- Compatibility impact: pre-release startup typestates, lock/rollback instrumentation, result owner,
  and evidence-query internals change; no stable report wire or compatibility adapter is preserved.
- Required review: ledger-first/physical-second barriers, typed no-mutation rejection rollback,
  post-mutation/panic/uncertain coupled containment, no nested locks or pre-ready allocator entry,
  external acceptance visibility, mapping-owner drop order,
  post-plugin-unload query, cross-domain key rejection, and brand-exclusion transcript/digest tests.
- Rationale: attribution and physical lifetime answer different questions and cannot truthfully share
  one snapshot. Atomic API acceptance comes from private ownership and rollback, while queryable inert
  keys remain meaningful only when Baozi owns their mapping for the same lifetime.

### 2026-07-19: Supersede retained-builder fallback with exact startup ownership

- Affected headings: `Lifecycle`, lifecycle diagram, `Prepared Outcome Publication`, `Risks and Mitigations`,
  `Success Metrics`, and `Acceptance Criteria`.
- Previous rule: the earlier 2026-07-19 amendment allowed a caught retained-builder unwind to publish
  minimal fallback after generic verified cleanup and did not distinguish startup from later retained
  construction in the import lifecycle.
- New rule: retained startup builder panic returns `SessionStartError` with no accepted session or
  terminal; successful startup supplies the exact baseline required for later downgrade; and later
  retained candidate/report builder panic returns `FinalizationContainmentError` without fallback.
  ADR 0042 exclusively owns the physical admission, side disposition, cleanup evidence, attribution
  coupling, membership, and release protocol below these import outcomes.
- Compatibility impact: pre-release session-start/finalization errors, state machine, and fault
  evidence change without a compatibility adapter or stable report wire.
- Required review: session acceptance, startup-versus-later builder failure mapping, no-terminal
  startup failure, fallback exclusion, lifecycle diagram, ADR 0042 integration, and ADR review.
- Rationale: an import may promise exactly one terminal only after startup owns a provable cleanup
  graph; a builder panic cannot be rewritten as an ordinary report when its closure may have escaped
  storage.

### 2026-07-19: Complete fallback evidence and account-poison containment

- Affected headings: `Decision`, `Lifecycle`, state diagram, trusted-extension boundary,
  `Prepared Outcome Publication`, `Risks and Mitigations`, `Success Metrics`, and
  `Acceptance Criteria`.
- Previous rule: explicit account poison named a fatal outcome without defining clone-shared state,
  operation-start ordering, post-admission finalization, or account reuse. Bootstrap described a
  complete dynamic fallback without a checked dimension-plus-
  phase cell layout, without key lifetime/normalization rules or complete operation-wide global
  evidence, and without distinguishing semantic aggregate preflight from actual allocation plans. A
  retained-builder panic always appeared to publish minimal fallback with zero live attribution.
- New rule: bootstrap checked-converts counts, creates sealed key-bearing dimension and phase cells
  with `Layout::array`, extends them after the complete global-evidence header in order, and calls
  `pad_to_align`. The resulting layout is semantic aggregate preflight only; every actual header or
  keyed-evidence backing has an independent policy-governed `CapacityPlan`, while a separate source
  evidence anchor pairs the concrete-store and policy revisions, and bootstrap checked-
  sums and atomically admits every requested envelope/block before allocator entry. Keys are inert
  Baozi-owned copied domain values normalized before the first ledger entry, so evidence remains
  queryable after plugin unload. The header stores, rather than derives, operation-wide current/peak
  live, active-reservation, and current/peak evidence-storage facts; prepared fallback verifies all
  current values are zero while preserving peaks. Every arithmetic/layout/admission failure returns
  `SessionStartError` before an accepted session. A caught single
  unwind publishes fallback only after complete cleanup and zero live attribution are proved.
  Explicit-account start and final publication eligibility linearize against ADR 0042's shared
  monotonic poison latch. Poison-first start rejects without work/ledger/IO/refcount/allocator effects;
  start/retain-first preserves charge but final preparation returns fatal containment and creates no
  exact wrapper. All clones share the latch/count/evidence and no reset/reuse/bypass exists. Uncertain
  in-process release keeps payload/lease/operation attribution coupled and charged, poisons the
  affected path, and returns fixed typed `FinalizationContainmentError` without report, fallback,
  refund, or zero-leak claim. Abort/double panic/exit/OOM require process isolation, as does any
  hostile/opaque decode that claims a hard runtime memory bound.
- Compatibility impact: the pre-release terminal API gains typed poisoned-account start rejection,
  shared poison snapshot evidence, a fatal containment result, and complete key-bearing fallback
  evidence/admission contract; no stable report ABI or in-memory wire is preserved.
- Required review: exact/excess/count/layout/envelope/block-overflow bootstrap tests, key
  normalization and post-unload query tests, global-field snapshots, state-machine/result exhaustivity,
  verified-unwind cleanup, clone/reuse and two-order start/retain/finalization poison barriers,
  zero-side-effect/no-bypass checks, uncertain-release poison/no-refund probes, process-isolated failures,
  hostile opaque-memory wording/gates, and direct ADR review.
- Rationale: exactly-one result does not justify fabricating a clean failure report when ownership
  release is unknown, and a complete evidence envelope must retain keys, phase data, and non-derivable
  operation-wide facts without pretending one aggregate formula is one allocator request. A shared
  account with one unverified release is no longer a valid admission domain for another import.

### 2026-07-18: Classify exact import terminals before pure handoff

- Affected headings: `Decision`, `Lifecycle`, the state diagram, `Prepared Outcome Publication`,
  `Success Outcome`, `Failure Outcome`, facade bootstrap, `Crate Error and Capability Ownership`,
  `Risks and Mitigations`, `Success Metrics`, and `Acceptance Criteria`.
- Previous rule: import/core were planned to consume an operation generic retained allocation through
  `AccountedAllocation<T>` and `CanonicalAllocationGuard`, prepared outcomes could retain pending
  attribution until handoff, and the draft/generation boundary was not tied to a checked pre-freeze
  identity-acquisition stage.
- Historical rule introduced here and superseded in its publication-boundary details by the
  2026-07-20 prepared-owner amendment: import and core can reach only the operation-owned concrete scalar/bytes/record store;
  core's `CanonicalBlockRef<Schema>` is a non-owning projection over a sealed unstable schema;
  `SceneDraft` is a brand plus directory-slot descriptor. Preparation completes final control
  downgrade and returns exact success, rich-failure, or minimal-fallback wrappers after freezing
  private canonical/outcome attribution at zero operation live. Only exact success can hold a private
  never-reused generation reservation; both failure wrappers structurally contain none. Reservation
  rejection/exhaustion consumes no nonce and becomes ordinary rich failure with preserved evidence;
  only failure to append/validate/prepare that rich failure or an unrecoverable builder/root integrity
  fault selects minimal fallback. This intermediate model let type-specific handoff reclassify
  attribution into `CommittedCanonicalOutput`/`PublishedOutcomeEvidence` and construct
  `SceneGenerationId`/`ValidatedScene` only for exact success. A long-lived bounded facade runtime
  owns one factory and private per-import accounts. The current rule applies destination commitments
  during preparation; handoff only exposes/moves the already qualified owners and constructs the
  success-domain values.
- Compatibility impact: the pre-release generic/owning wrapper plan, handoff-time accounting,
  post-freeze generation allocation, prepared-token rollback behavior, global/per-request facade
  factory paths, and legacy Scene/`SharedStorage` path are removed rather than adapted. No public wire
  compatibility is promised for concrete-store layout.
- Required review: exact-terminal importer typestate and crate-boundary review, generic-authority compile-
  fail gates, structural-preflight and builder/root fault injection, generation exhaustion/never-
  reuse/prepared-drop probes, rich-failure-after-reservation-fault and fallback-after-rich-preparation-
  fault probes, facade profile/factory lifetime tests, success/rich-failure/fallback
  snapshots, full workspace migration, and direct ADR review.
- Rationale: an import outcome is atomically publishable only when physical ownership is provable and
  every accounting failure has ended before the final ownership move and generation identity exist.

### 2026-07-17: Make exact importer invocation a lifecycle-owned downstream contract

- Affected: `Status and Authority`, `Registered Importer Invocation Contract`,
  `ParseScope Capabilities`, `Parse Failure Transport`, and `Crate Error and Capability Ownership`.
- Previous: registry governance and import lifecycle both described detector/parser invocation and
  failure transport, while the ledger ADR treated the lifecycle mapping as a normative upstream
  input. That made the prose dependency graph cyclic even when frontmatter appeared acyclic.
- New: ADR 0021 owns registered callable identity, slot lifetime, health observation, and poison;
  this ADR alone owns the exact detector/parser method shapes, invocation scopes, domain transport,
  validation call, and import outcome composition. ADR 0022 supplies generic owner-defined ledger
  mechanisms without consuming this lifecycle.
- Impact: the Accepted authority graph is now one-way: registry, ledger, IO, source-scope, and
  resource-graph contracts feed this downstream import orchestrator. No duplicate invocation owner
  or reverse lifecycle dependency remains.
- Review: 2026-07-17 ADR authority-cycle and implementation-boundary review.
- Rationale: callable registration and callable invocation have different reasons to change; the
  lifecycle can consume a healthy registered slot without making registry or accounting depend on
  import outcome semantics.

### 2026-07-17: Make outcome preparation fallible and ownership handoff infallible

- Affected: `Lifecycle`, state diagram, `Resource-Graph Evidence Transport`,
  `Prepared Outcome Publication`, `Success Outcome`, `Failure Outcome`,
  `Crate Error and Capability Ownership`, `Risks and Mitigations`, `Success Metrics`, and
  `Acceptance Criteria`.
- Previous: the single finalizer promised atomic outcomes but did not distinguish all fallible
  preparation work from the final ownership handoff or give retained failure evidence an independent
  ledger destination.
- New: the session pre-admits a minimal account-bound terminal envelope; preparation performs every
  fallible action and returns a sealed linear prepared terminal state; handoff is infallible; and
  outcome-evidence attribution remains independent of canonical-output attribution. The 2026-07-18
  amendment refines that state into exact success/rich-failure/minimal-fallback wrappers and reserves
  `PublishedOutcomeEvidence`/`CommittedCanonicalOutput` names for handoff results.
- Impact: the pre-release finalizer and report/failure ledger shape change; a failure outcome may
  publish evidence while canonical committed output remains zero.
- Review: 2026-07-17 lifecycle, storage-owner, terminal-outcome, and failure-publication atomicity
  review.
- Rationale: a one-finalizer contract is atomic only when no allocator, validator, account transition,
  control poll, callback, or other failure source remains after preparation succeeds.

### 2026-07-16: Carry origin-qualified resource graph evidence through one finalizer

- Affected: `Decision / Lifecycle`, state diagram, `Decision / ParseScope Capabilities`,
  `Decision / Resource-Graph Evidence Transport`, `Decision / Success Outcome`,
  `Decision / Failure Outcome`, `Risks and Mitigations`, and `Acceptance Criteria`.
- Previous: the lifecycle exposed classified dependency acquisition and extensible success/failure
  evidence, but did not define the concrete capability and atomic outcome slots for an origin-
  qualified graph, successful input manifest, or bounded partial graph.
- New: one private session graph builder supplies opaque origins through `ParseScope`; staging carries
  ADR 0010's atomic failure-plus-proof/none result without post-cache upgrades; the sole finalizer
  atomically publishes complete success evidence or distinct bounded partial failure evidence while
  ADR 0041 remains the graph/manifest semantic owner and Proposed ADR 0043 stays outside production.
- Impact: closes parser bypass and partial-publication ambiguity with a pre-release capability/report
  shape change; no stable public compatibility promise exists.
- Review: 2026-07-16 lifecycle/graph integration review, independent Codex architecture and security
  reviews, and the repository architecture review.
- Rationale: this extends ADR 0020's existing session capability and outcome-transport concern; it
  does not move graph semantics into the lifecycle ADR or replace the selected single-finalizer
  architecture.
