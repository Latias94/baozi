# Tiny STL: locating fixed import costs

This is an additional locating measurement on the original 134-byte
`stl-repeated-1` task, after both glTF field-layout native attempts terminate.
It does not replace the six-task instruction selection or any missing native
campaign. No optimization, native-speed, heap or release conclusion follows.

## Ordinary observations and question

The [partial native attempt](2026-11-08-gltf-field-layout-native.md) retains
one Windows candidate campaign and two controls. For STL1/memory, import
stage medians are 1,074,800 ns / 1,072,350 ns / 1,089,250 ns across C0/N0/C1;
same-campaign Assimp medians are 76,350 ns / 76,050 ns / 75,400 ns.
These ratios are approximately 14.08x / 14.10x / 14.45x.

The candidate's prepare median is 110,400 ns against Assimp 1,500 ns, and
extract is 12,750 ns against 1,050 ns. Those are separate stage tasks: never
sum their medians to reconstruct pipeline. The single candidate, unequal
sampled load and absent N1 prevent source-effect attribution. They nevertheless
identify fixed import work as worth locating before another architecture choice.

## Exact diagnostic and scope

One fresh GNU Callgrind process runs the existing 7b8 symbol worker:

- Source `7b8c5f3d2559085967dcdfd4098130c77bd877ce`.
- ELF SHA-256 `14c41c745814ee2116b55691d1b649e86f1120f2dfef5b22733bfa0048a14ef4`.
- The same qualified Rust 1.95 opt3/debug1, generic CPU/System profile.
- Original pipeline/memory task, position fields, preserved normals and
  max_work_units 200,000,000; original corpus manifest and input hashes.

The worker and two annotations complete successfully. Its semantic observations
match the original task. Whole-worker Ir is **2,140,762**, including loader,
runtime setup, import, prepare, extraction, observation, serialization and
teardown. It is not an import-stage instruction counter or native timer.
Raw data remains under
`target/assimp-comparison/gltf-field-layout-small-stl-attribution-1`.

## Locating observations

The retained inclusive annotation reports the following named entries. These
overlap, include callees and inline contexts, and must not be summed:

| Named entry / context | Inclusive Ir |
| --- | ---: |
| `ImportPlan::read_bytes` | 1,463,090 |
| `session::run_import_session` | 1,411,971 |
| `ResourceGraph::seal`, outlined source context | 413,127 |
| `import_ledger_limits`, outlined entry across all calls | 149,718 |

The resource-graph entry also has an inline source context at 416,981 Ir;
that is not an independent additional cost. No label is interpreted as wholly
removable framework overhead.

Three disjoint physical Self rows attributed to SHA2's software compression
contain 155,385 + 130,869 + 80,586 = 366,840 Ir. Callgrind's executed software
backend does not establish the native runtime-dispatch backend or native-time
proportions. Loader and libc activity also remain in whole-worker totals.

The original raw call arcs independently identify three actual calls to the
same outlined policy constructor, with their inclusive costs:

| Caller | Calls | Inclusive Ir |
| --- | ---: | ---: |
| `session::import_bytes` | 1 | 50,958 |
| `ImportStoreBootstrapRequirements::for_node_scene` | 1 | 49,480 |
| `try_import_store_bootstrap` | 1 | 49,280 |

Their sum is 149,718 Ir, already the entry above. It is not added again, and
does not predict the saving from removing a construction. A separate source
trace is investigating whether the latter callers can borrow the original
validated policies while preserving startup ownership and failure ordering.
No request-global cache, shared mutable account or replacement counter is
proposed by this locating measurement.

The bounded source trace independently confirms three constructions on each
successful public read path. Preparation validates and discards the first
import-typed owner; requirements and bootstrap each reconstruct it. Each owns
four Vec buffers across 58 dimensions. The final Store-keyed 62-dimension
limits are a distinct genuine owner and must still be constructed.

The narrow next design retains the entry's original validated owner, borrows
it for requirements, then moves it into bootstrap for policy mapping. No clone
or long-lived request cache is needed. This changes temporary lifetime and
removes two allocation-failure opportunities; identical OOM occurrence is not
claimed. Surviving typed startup failures, deterministic precedence, checked
budget partitions, quotas and final physical ownership remain obligations.

## Qualification and next decision

Independent raw profile/annotation/stream and direct-call evidence review
closes with GO. The unique summary, footer, stderr, raw Self sum and both
annotation totals reproduce, as do all six stream hashes, original task
facts, actual binary/source binding and three incoming call arcs. No warning
occurs in this worker; earlier grid warnings remain separate retained facts.
Supervisor terminal0 is coordinator tool evidence without a separately
retained supervisor exit record. Full runtime environment and host isolation
are not independently attested. One successful worker is a locating
observation, not a paired optimization result. There is no Assimp instruction
run here.
Original Windows/GNU timing, memory and complete comparison requirements
remain active; no favorable profile can repair their missing campaigns.

Before implementation, complete the concrete source-ownership trace and judge
the smallest design that eliminates demonstrated repeated work. A different
short-loan parser success-budget design is independently reviewed but has not
been implemented. Choosing between these opportunities depends on the located
costs and their actual semantic/ownership obligations, not on an assumed streaming
advantage or a blanket removal of checks.
