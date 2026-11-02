# Assimp 6.0.5 Progress and False-Return Observations

The fixed Windows and Linux diagnostic runs confirm that returning `false` from
`ProgressHandler::Update` does not cancel the selected OBJ, STL, PLY, glTF2 and common
postprocess paths in Assimp 6.0.5, revision
`392a658f9c271be965271f45e7521a1b80ea4392`. The library calls the void
`UpdateFileRead`/`UpdatePostProcess` adapters, which discard that boolean. The diagnostic
preserves this path: it neither throws from the callback nor fails IO to force an abort.

This qualifies the named release, cases and public call sequence. It is not a claim about
every Assimp release, importer, external wrapper or possible cancellation mechanism.
No timings or memory comparisons are collected by this instrumented executable.

## Fixed Matrix and Results

Each host performed 72 fresh imports, eight additional public smooth-normal postprocess
calls and three standalone progress-adapter calls, producing 75 records. The eight original
inputs contain the same analytic triangle: short/padded OBJ, ASCII/binary STL, ASCII/binary
LE PLY, embedded JSON glTF and GLB. Each uses both file and exact-primary memory IO.
The padded OBJ has 33.5 MiB of bounded comment lines to cross the library's 16 MiB cache.
Every record checks actual geometry, normals, callback route and return policy, IO behavior,
handler ownership and bounded trace completeness.

| Observation | Windows | Linux |
| --- | ---: | ---: |
| Baseline success without a false-return request | 18 | 18 |
| Selected callback returned false; geometry still completed | 40 | 40 |
| Requested strict interior callback did not occur | 14 | 14 |
| Standalone adapter/default-argument checks | 3 | 3 |
| Unrelated failure / incomplete trace | 0 | 0 |
| All individual checks passed | 75/75 | 75/75 |

The 14 missed triggers are explicit coverage limits. A baseline does not turn a later null
scene into proof of cancellation; null-after-false would remain an unattributed failure.
Neither host produced a null result in this matrix. Both retained full valid geometry after
each selected false return, including the first and final read callbacks and the first,
interior and final postprocess callbacks.

Short OBJ supplies three file-read callbacks; padded OBJ five; STL, PLY, JSON glTF and GLB
each supply only the two common read boundaries in these runs. Smooth-normal cases add 34
registered-step callbacks, including inactive registered steps. These counts describe the
observed routes, not a promised progress cadence within a parser or normal kernel. OBJ's
cached bytes can reach the input size before parsing/native construction has finished.
Zero optional postprocess flags can finish with the mapped read value 0.5.

Direct `Update()` observes the documented unknown sentinel -1; the two zero-total adapters
map to 0.5 and 1.0. Those adapters do not emit unknown. A progress UI must not interpret a
cached-read fraction as complete semantic parsing or infer uniform intermediate coverage.

For example, first-read false on short OBJ still precedes 92 bytes of further reads and
two callbacks. First-postprocess false still precedes 33 callbacks and a valid generated
positive-Z normal stream. Final-callback false leaves zero later callbacks but still returns
the valid scene. These observations distinguish effective cancellation from callback presence.

## Qualification

Windows uses MSVC 19.51, x64 Release, static Assimp/bundled zlib and dynamic CRT. Linux uses
Clang 21.1.8, x86_64 Release with symbols and static Assimp. Neither modifies the upstream
library. The retained archive SHA-256 is
`8241204095ad8b6e6898fa6337b345c72b164ecca17a981a95bcaa81b7185948`; the 1,651 archive files
and separately recorded pkg-config input reconcile all 1,652 source files. The Linux run
freshly rechecked each against the retained source manifest; the source audit did the same
for the Windows extraction. Archive-declared identity remains distinct from Git verification.

The callback uses a fixed 256-event array and does not allocate, throw, call the importer or
perform IO. Trace overflow would preserve decisions while failing qualification afterward.
Input buffers, counters and trace outlive importer-owned handlers. Actual JSON was parsed
after each run, and additional owner-side checks reconciled all 75 records, callback ordinals,
the I3 baseline-final selection, continued work after false, generated normals and adapter values.

Five negative CLI cases pass on each host: wrong arity, existing root, existing report, report
outside root and absent parent. Exit code 2 is retained and existing sentinel files remain
byte-identical; refused paths are not created. The Windows F64 build separately refuses this
F32-only matrix before creating its root. The immutable ordinary F32 comparison binary is
unchanged. These are argument/artifact checks, not a hostile-filesystem security qualification.

Independent source review found no actionable issue in callback propagation, noexcept/storage,
lifetimes, IO forwarding, analytic fixtures, classification or finite matrix accounting.
It checked the relevant 23 upstream file hashes. Runtime results then matched the source
prediction on both hosts. Source review alone was not treated as a runtime result.

## Retained Evidence

Diagnostic implementation: `0936bd83d4eacf688491196022313f94dc3a87bc`. The committed C++ and
CMake file hashes match both executed builds. Exact executable/build/source/static-library
hashes and commands are retained in `target/review/p6-assimp-control-{windows,linux}/`.

| Host | Report SHA-256 | Executable SHA-256 |
| --- | --- | --- |
| Windows | `691f9f3b1b8026d4f651df1b2275a745c17b5221c6864c5de6a1ed5a405ec980` | `2103bf785c4539c3da94f19021f2a789f5f0e9b9c988994f41026fbf0b80b2d7` |
| Linux | `943c6dfc9e72702d7a578c98ec1c2d80cd869934aa5316d316a3b5ef7278e334` | `ca45b08b36396d417be5de09676950eb221be0ba7b9599eb8b86210383de2ccd` |

Reports and all generated inputs remain in
`target/assimp-comparison/p6-control-{windows,linux}-f32/`; source and independent reviews
are retained with the 61-file verification archive `target/review/p6-assimp-control-evidence.zip`,
SHA-256 `7d8a79315903b5cf8a216641667d00811fe81f78e5f7cd67acf2ac68563d106f`.
The [diagnostic guide](../../tools/consumers/assimp/CONTROL.md)
defines the exact triggers, IO path and schema. This completes the bounded Assimp control
observation slice. Baozi's real-format cadence/progress matrix, P5 cost attribution, full B1
and final U14 remain open. No publishing, push or native promotion follows.
