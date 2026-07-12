# Scale Profile Policy

Baozi separates deterministic resource correctness from statistical performance evidence. The
repository gate does not treat wall-clock timing as a pull-request correctness oracle.

## U3 Profile

The U3 profile uses one Baozi-owned generated OBJ case, `obj_repeated_triangle_faces`. It contains
three fixed vertices followed by repeated fixed-width triangle records. A logical 1x fixture has 256
faces. The correctness suite imports 1x, 10x, and 100x. The scheduled profile additionally imports
the selected 1000x case.

The generator lives in `baozi-test-support`; generated assets are not checked in and have no
third-party provenance or redistribution requirement. Fixture construction reserves its final byte
capacity before filling it so generator reallocation does not become parser evidence.

## Deterministic Gate

Run the local correctness gate with:

```powershell
cargo +1.95.0 nextest run --locked -p baozi --test import_scale_contract --all-features
```

For every correctness scale, the test requires affine OBJ face, record, token, and primary-byte
counters; the expected canonical index count; and a clean terminal ledger. A clean terminal ledger
has zero current live bytes, active reservations, ledger evidence bytes, phase-owned live bytes, and
pending publication. Consumed work cannot exceed attempted work.

Declared peak-live memory must remain below this reviewed affine envelope:

```text
8 MiB + 16 * input_bytes + 2048 * face_count
```

The same constants are duplicated deliberately in `ci/scale-profile-policy.json` and the typed
fixture implementation. The scheduled runner rejects disagreement. Changing them requires a review
of the generated fixture, ledger ownership, and fresh scale evidence; a prior CI artifact is never a
correctness baseline.

The Criterion harness can be checked without collecting measurements:

```powershell
cargo +1.95.0 bench --locked -p baozi --bench import_baseline --all-features -- --test
```

## Runner Contract Gate

Pull-request CI parses `scripts/run-scale-profile.ps1` with PowerShell's AST parser and runs its
`-ValidateOnly` mode. This mode reads the production `ci/scale-profile-policy.json` through
`ci/scale-profile-policy.schema.json`, constructs the exact Criterion, probe-build, and GNU time
argument arrays used by scheduled collection, and exercises the production probe, unsigned-counter,
GNU time RSS, bounded command-output, report-construction, JSON round-trip, and
`ci/scale-profile-report.schema.json` paths with synthetic evidence. Hostile duplicate or overflowing
probe values, malformed RSS evidence, zero-byte output limits, and command output beyond either
stream limit must be rejected for the gate to pass. Output exactly at the configured byte boundary
remains valid.

Validation writes `validation.json` with `execution_mode: validate_only`. It launches short instances
of the current PowerShell executable solely to verify stdout/stderr enforcement; it does not run
Cargo, Criterion, the compiled probe, or GNU time and is never scheduled scale evidence. Only the
weekly or manually dispatched full collection can produce `report.json` with
`execution_mode: collection`.

## Scheduled Evidence

`.github/workflows/scale.yml` runs weekly and on manual dispatch on a fixed Ubuntu 24.04 runner. Its
PowerShell runner reads `ci/scale-profile-policy.json` from the repository. The workflow has no
environment variable or input that can replace the policy or its thresholds.

Criterion 0.5.1 records time distributions for all four scales. The runner resolves locked Cargo
metadata and fails unless the single resolved `criterion` package is exactly 0.5.1; the policy value
is therefore checked, not merely copied into the report. Median time and its confidence interval are
normalized into Baozi's report schema. A missing case, missing or non-finite estimate, or wrong sample
count fails the workflow. Iteration and time series must both be finite, positive, equally sized, and
match the policy sample count; confidence bounds must contain the median point estimate. A per-decade
power exponent above 1.25 is an advisory trend regression in U3 and is recorded without turning
wall-clock variance into a hard correctness failure.

The independently built `import_scale_probe` benchmark executable imports each fixture once. GNU
time runs that executable in a separate process three times per scale and records the largest maximum
resident set size. An idle process with the same default registry supplies the fixed-process baseline.
The hard delta conservatively subtracts the smallest idle RSS from the largest scale RSS; a noisy
high baseline cannot hide scale growth. RSS is external process evidence; it is not called heap
allocation count and is not substituted for ledger evidence. Every repetition must emit the expected
schema, case, numeric scale, and scale label, and all non-RSS probe values must agree across
repetitions.

The policy gives tool identity, Cargo metadata, Criterion, probe build, and individual probe
processes independent timeouts. Every command also has a 16 MiB limit for stdout and a separate
16 MiB limit for stderr. Both streams are copied asynchronously from the process pipes directly into
artifact files; they are decoded only after the command finishes and both files satisfy their limits.
A timeout or output-limit breach kills the command process tree. An over-limit artifact is truncated
to the configured boundary before the runner fails closed, so failure evidence stays bounded. These
limits are repository policy and cannot be replaced by workflow inputs. A 40-minute script-wide
deadline caps their cumulative runtime before the collection step's 45-minute timeout. The 75-minute
job timeout and 15-minute deterministic-check timeout leave a separate tail for setup and
failure-artifact upload.

RSS checks are hard failures:

- maximum process RSS must not exceed 1 GiB;
- RSS above the idle probe must fit `input_bytes + 1.25 * ledger_peak + 16 MiB`;
- each tenfold scale increase must fit `12 * previous_delta + 8 MiB`.

Missing GNU time, a probe failure, a timeout, missing RSS, a schema mismatch, or a missing artifact
fails closed. Thresholds are intentionally coarse security and complexity envelopes, not microbench
regression percentages.

## Artifact Contract

The normalized artifact schema is `baozi-scale-report/v1` and is machine-defined in
`ci/scale-profile-report.schema.json`. `report.json` records `execution_mode: collection` and:

- profile, policy SHA-256, GitHub run identity, Rust, Cargo, Criterion, and GNU time identity;
- Ubuntu runner-image identity, kernel identity, logical processor count, and structured `lscpu`
  evidence;
- case ID, scale, logical units, input bytes, Criterion median/confidence/sample count;
- every scale and idle RSS repetition, their min/max aggregates, and the conservative delta;
- ledger peak live/evidence bytes, reviewed envelope, and committed canonical output;
- every hard and advisory check with observed value, limit, unit, and result.

Criterion runs below the runner's temporary directory, outside the uploaded artifact root and Cargo
target cache. Only each scale's `estimates.json` and `sample.json` are copied into the artifact, and
the runner rejects any HTML file or unexpected Criterion artifact. Probe output, GNU time output, and
command logs accompany the normalized report. A script bootstrap or collection failure writes
`report.json` with `status: error`, the failed step, and bounded exception identity/message. A final
`always()` workflow step validates that report against the repository schema and creates the same
minimal error schema when checkout,
the deterministic contract, or collection failed before the script could do so. The workflow uploads
evidence even after failure, rejects an empty artifact set, and retains artifacts for 90 days.
Generated fixture bytes and Criterion HTML are not uploaded.

## Future Units

U4 through U7 add graph validation, canonical IR, format, glTF admission, remap, and postprocess cases
when those behaviors land. glTF admission still requires its own bounded subprocess RSS and timeout
proof; this trend workflow cannot waive it. U8 completes million-cell and selected 1000x snapshot and
semantic coverage. U9 makes workflow, feature, maturity, and evidence registration machine-checked;
it does not defer the first scheduled scale run until release work.

Criterion is already an `Apache-2.0 OR MIT` development dependency. The profile adds no allocator or
registry dependency. GNU time is an external CI executable and is neither linked into nor distributed
with Baozi.
