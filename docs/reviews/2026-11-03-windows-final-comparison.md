# Final-Source Windows Ordinary Comparison

Runtime implementation `52446ec8511b7dfba2e5352ce6941558d85e0ae2` completes the unchanged
Windows comparison matrix: **110/110 lanes, 2,200 retained successful pairs and 330 warmup
pairs**. All 5,060 child observations are retained. This is the Windows portion of P6, not
Linux B1, heap attribution, final U14 or release qualification.

The fourteen original cases, file/memory modes, admitted endpoints, precision, selected fields,
normal policy and analytic semantic checks are unchanged. Each lane has three warmup pairs
then twenty alternating AB/BA pairs. F64 PLY uses the retained double-precision Assimp binary.
There is no allocation interception, profiler or diagnostic instrumentation in ordinary workers.
Builds are outside collection. The immutable Windows worker and Assimp controls use the same
ordinary build policy as the historical series at `4455d03db190c6bef05ff5a1de024d2aac461465`.

## Complete Pipeline Examples

These are memory-input pipeline medians, milliseconds. The artifact preserves all import,
preparation, extraction and file-input observations, with IQR and every raw sample. Separate
endpoint medians are not summed/subtracted as an exact decomposition of the pipeline.

| Case | Baozi control | Baozi current | Assimp current |
| --- | ---: | ---: | ---: |
| Repeated OBJ 1 | 1.618 | 1.662 | 4.140 |
| Repeated OBJ 1,024 | 18.299 | 7.344 | 4.308 |
| Repeated OBJ 16,384 | 296.912 | 90.450 | 9.406 |
| Grid OBJ 2 | 1.406 | 1.460 | 4.019 |
| Grid OBJ 32 | 25.573 | 13.601 | 5.376 |
| Grid OBJ 128 | 393.785 | 194.923 | 24.758 |
| Shared glTF 1 | 2.197 | 2.022 | 0.255 |
| Shared glTF 8 | 3.328 | 2.746 | 0.275 |
| Shared glTF 32 | 7.978 | 5.168 | 0.333 |
| Binary STL repeated 1 | 1.261 | 1.266 | 0.113 |
| Binary STL repeated 1,024 | 6.615 | 4.488 | 0.228 |
| PLY triangle 1 | 1.317 | 1.333 | 0.585 |
| F64 PLY neighbors | 1.303 | 1.344 | 0.595 |
| PLY points 1,024 | 3.204 | 1.827 | 0.734 |

The large repeated/grid OBJ pipelines improve about 69.5%/50.5% against the historical control.
The current repeated import/preparation/extraction medians are 36.679/48.738/4.724 ms; current
Assimp is 7.491/1.028/0.386 ms. Current grid import/preparation/extraction is
179.001/0.151/14.966 ms; Assimp is 23.127/0.031/0.988 ms. The gap remains substantial.
These are finite semantically matched tasks, not arbitrary-model API or algorithm parity.

Small pipeline medians can move a few percent in either direction. The largest relative
stage increase is repeated-1 OBJ extraction, about 9.3 to 11.3 microseconds. The short grid
no-change preparation lane rises about 139 to 151 microseconds with about 12 microseconds
current IQR. Keep these observations; they do not support a broad no-regression or universal
speed claim. Linux's frozen B0 budgets are adjudicated separately rather than applied to
Windows working-set or timing data.

## Memory Scope

Whole-child memory-input pipeline medians, MiB:

| Case | Baozi peak working set | Baozi peak private commit |
| --- | ---: | ---: |
| Repeated OBJ 16,384 | 16.930 | 12.879 |
| Grid OBJ 128 | 32.164 | 29.527 |
| Shared glTF 32 | 10.297 | 1.281 |
| Binary STL repeated 1,024 | 9.164 | 1.275 |
| F64 PLY neighbors | 9.311 | 1.049 |
| PLY points 1,024 | 8.914 | 1.125 |

These peaks include input, startup, output, checking and serialization. They are neither Linux
RSS nor stage heap. The [bounded algorithm receipt](2026-11-03-source-remap-and-normal-ordering.md)
separately compares the same-source checkpoint before/after working set and private commit.
The full report retains Baozi and Assimp memory statistics for all 110 lanes; no missing metric
is converted to zero or a passing memory claim.

## Evidence and Remaining Work

- Complete report: `target/assimp-comparison/Windows-final/report.json`, SHA-256
  `4429aab5d79d6f3a364cc296f362a9308c2b8f85b5ed1f1e74c4dfb18f47305d`.
- Full control/current statistics: `target/review/final-windows-disposition.json`.
- Exact invocation/log: `target/review/final-windows-collector-argv.json` and
  `target/review/final-windows-collector.log`.
- Retained ordinary worker: `Windows-p5-algorithms-build/baozi-probe.exe`, SHA-256
  `708ce98698435a72e1e3274058c9b1ebba1c3bfb224b1aeef3b38722938a7333`; source/build manifest
  and source archive remain in that directory.

Independent static measurement review reports no actionable finding. It verifies all 110 keys,
all 5,060 successful child records, paired order/counts, analytic observations, recomputed
statistics, document tables and retained binary/source identities. It runs no new worker or
profiler. The artifact archive, Linux B1/heap and final integrated qualification are attached
separately. `target/review/final-windows-evidence.zip` retains 10,144 files, SHA-256
`2d62c9e8453ca148f18e1763c720e1ec61a18ee2a5fc59e2b4888350d349d3c0`.
No registry publication, target promotion or Assimp performance superiority claim
follows from this result.
