# Scale Profile

Baozi separates deterministic resource correctness from advisory performance measurements. Rust
tests and benchmarks own the fixtures and assertions. The Python driver only runs fixed commands,
keeps their output, and writes a short report; timing and RSS have no pass thresholds.

## Rust Workloads and Contract

`baozi-test-support::ObjScaleFixture` generates `obj_repeated_triangle_faces`: three vertices followed
by repeated triangle records, with 256 faces at 1x. `import_scale_contract` checks 1x, 10x, and 100x.
`import_baseline` and `import_scale_probe` also exercise 1000x.

The separate `unique_vertex_growth_contract` imports 256, 1,024, and 4,096 triangles with three
distinct, nonreused vertices each. Public typed reads check every position and index as well as
the complete geometry counts. Real `WorkUnits` must fit fourfold prior work plus the larger
input's heap-sort upper bound: `2m * (floor(log2(m)) + 1)` comparisons, rounded up to the existing
64-unit work block. The input uses fixed-width records, so bytes grow exactly fourfold. This
allows the actual position-normalization sort without a timing tolerance or a quadratic scan.
Terminal resource/vertex/topology counters and zero live bytes/reservations remain required.

The deterministic test checks affine primary-byte, record, token, and face counters, the expected
scene shape and index count, and a terminal ledger with no retained live bytes or reservations.
Consumed work must not exceed attempted work. Declared peak-live memory must fit the reviewed affine
envelope owned by the Rust fixture:

```text
8 MiB + 16 * input_bytes + 2048 * face_count
```

Run that test independently with the OBJ feature set:

```text
cargo nextest run --locked -p baozi --no-default-features --features format-obj --test import_scale_contract
cargo nextest run --locked -p baozi --no-default-features --features format-obj --test unique_vertex_growth_contract
```

The existing probe also checks terminal ledger state, scene shape, and the declared-memory envelope.
These Rust assertions remain correctness requirements. Declared ledger memory and external process
RSS measure different things and must be reviewed separately.

## Local Collection

Use Python 3.10 or newer, Rust, and cargo-nextest. The scheduled workflow pins Rust 1.95.0. No Python
packages are required. From the repository root, run:

```text
python scripts/run_scale_profile.py --help
python scripts/run_scale_profile.py
python scripts/run_scale_profile.py --artifact-root target/scale-artifacts/local-review
```

The script can also be invoked by absolute path from another directory. An artifact path is relative
to the repository unless absolute. It must be new; an existing directory is rejected to preserve
earlier measurements. The default creates a timestamped directory below `target/scale-artifacts/`.
Commands run sequentially in the repository and reuse Cargo's normal target directory.
`CARGO_BUILD_JOBS` defaults to 1 when unset.

The driver runs both deterministic nextest targets, the `obj_face_import_scale` Criterion group, and a
build of the existing `import_scale_probe` benchmark. Every Cargo invocation uses `--locked`,
`--no-default-features`, and `--features format-obj`. Criterion uses a one-second warm-up, three-second
measurement, 20 samples, and 2,000 resamples. Cargo's JSON build output supplies the probe executable
path. The driver then invokes that executable directly once for idle and once for each scale.

On Linux with `/usr/bin/time`, each probe is a separate measured child, invoked with an argument
array. GNU time's `%M` value is converted from KiB to bytes. It excludes Cargo and compilation, but
includes the probe's registry, fixture construction, import, and scene snapshot. Idle RSS is recorded
separately; the driver does not subtract a baseline or enforce a growth envelope. On other platforms,
or when GNU time provides no positive peak, `peak_rss_bytes` is `null` with an unavailable reason.
Missing RSS does not turn a successful Rust process into a failed correctness check.

Each subprocess has a timeout: 900 seconds for nextest and Criterion, 600 seconds for the probe
build, and 180 seconds per probe. The driver uses Python's standard subprocess timeout handling;
it does not implement process-tree management or output polling. Output streams go directly to files
without an in-memory capture or a custom byte-limit protocol. A launch error, nonzero process exit,
or timeout stops collection, writes the partial report, and returns a nonzero exit code.

## CI and Artifacts

Pull-request CI compiles the Python script and checks `--help`. It does not run a synthetic validation
mode. `.github/workflows/scale.yml` runs the complete driver weekly and on manual dispatch on
Ubuntu 24.04, with a 60-minute collection limit and a 75-minute job limit.

Each run directory contains:

- `report.json`: completion or failure, timestamps, executed argument arrays, timeouts, return codes,
  command elapsed times, log paths, and each probe's measured peak RSS or unavailable reason;
- `<command>.stdout.log` and `<command>.stderr.log`, including Cargo build messages and the existing
  Rust probe output;
- `probe-<scale>.rss-kib.txt` when GNU time is used, including the separate idle observation;
- `criterion/`: Criterion's original measurements for the selected benchmark group.

`status: completed` means all commands succeeded. It makes no timing or RSS pass claim. Command
elapsed time includes build and harness overhead; inspect Criterion's own measurements for import
timing. Compare results in the context of the host, toolchain, and run variability.

The workflow uploads available artifacts even after collection fails and retains them for 90 days.
Failures before the driver starts use GitHub Actions' step logs; the workflow does not fabricate a
measurement report. The legacy `baozi-scale-report/v1` report schema and PowerShell `ValidateOnly`
contract are retired. The driver does not read `ci/scale-profile-policy.json` or its schema: Rust owns
the correctness constants, and the driver contains only fixed collection settings.
