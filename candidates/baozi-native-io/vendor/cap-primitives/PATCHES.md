# Baozi bounded manual-link patch

## Provenance

This directory is the full crates.io `cap-primitives` 4.0.3 package, copied from
the local Cargo registry on 2026-10-31. All 168 original package files were
verified byte-for-byte against the cached `.crate` before patching. No source,
build script, README, license, manifest or bundled lockfile was pruned. Registry
cache markers `.cargo-ok` and `.cargo-checksum.json` are omitted.

- Upstream: https://github.com/bytecodealliance/cap-std
- Upstream commit: `b7acf8e8807fe3fab991884d2208b7e03d35a409`
- Upstream package path: `cap-primitives`
- Original package version: `4.0.3` (unchanged)
- Original registry source: `registry+https://github.com/rust-lang/crates.io-index`
- Original package SHA-256:
  `8b5f74729fd2f44701d1a8eb47e906cdb3ccd9ec0f02baad85a744b791940b18`

The original `.cargo_vcs_info.json`, copyright and license files are retained.
The checksum above identifies the unmodified upstream archive, not this patched
directory. Local changes use the crate's existing license options.

## Changes

- `src/fs/open_bounds.rs` exports the existing 40/63 expansion limits, the
  existing four Linux fast-path attempts, a new 4096 native-unit manual target
  limit, and checked `open_work_bound(u64, u64) -> Option<u64>`. The platform
  constants and Linux loop use these shared values rather than duplicates.
- `src/fs/manually/read_link_one.rs` checks target length before the existing
  resolver builds target components. Unix units are bytes; Windows units are
  UTF-16 code units, including unpaired surrogates. Over-limit targets are
  `InvalidData` errors. Containment, parent handling and link counts are unchanged.
- `src/rustix/fs/read_link_unchecked.rs` replaces `rustix::readlinkat`'s allocation
  growth/retry loop with one `readlinkat_raw` call into a fixed 4097-byte buffer.
  The sentinel permits exact 4096-byte targets and rejects longer/truncated
  targets. Only bounded returned bytes enter the reusable `PathBuf`.
- `src/windows/fs/read_link_unchecked.rs` replaces the manual resolver's
  handle-to-ambient-path/`std::fs::read_link` helper with one same-parent relative
  reparse open and one fixed-buffer `FSCTL_GET_REPARSE_POINT` query. It checks the
  common header, declared payload, tag, flags, both names' even offsets/lengths,
  ranges, target limit and NULs before conversion. Parsing uses safe byte slices,
  not unchecked `winx::read_link` pointers. Absolute targets remain rooted and
  are rejected by the existing resolver; relative spelling is preserved.
- Both manifests add Windows binding features `Win32_System_Ioctl` and
  `Win32_System_SystemServices`, used for the query and tag constants, and narrow
  the upstream `windows-sys` requirement from `>=0.60, <0.62` to `=0.61.2`.
  This preserves the existing reviewed binding version: offline resolution of
  the local patch otherwise selected 0.60.2. It is not an upstream-version
  upgrade. The `cap-primitives` package version and other requirements are
  unchanged.
- Unit tests cover checked work envelopes and malformed/exact/excess native
  reparse records. No new resolver or ambient reopen fallback is introduced.
- `src/fs/manually/open.rs` drops upstream's unused `std::borrow::Cow` import.
  This is a trivial lint cleanup with no behavior change.
- `src/fs/read_link.rs` keeps all five upstream Unix link-content test cases but
  creates their owned temporary directory with `std`. The published upstream
  manifest removes its path-only `cap-tempfile` dev dependency, so the original
  test otherwise cannot compile from the complete registry package. No normal
  dependency, production implementation or tested behavior changes.

## Abstract work envelope

The helper applies to the existing normalized relative UTF-8 read-only asset
open on Linux, Windows and macOS, without `racy_asserts` or custom `O_PATH`.
The wrapper must supply the actual input length `P` in UTF-8 bytes and count `N`
of normal components, and use that exact path and the documented options.
It does not register additional Baozi targets. Other platform paths, notably
FreeBSD's runtime probe, return `None` rather than reuse this formula.

Let `S` be the platform's expansion count, `T = 4096`, `R` the fast-path attempt
count, `B` the fixed link-read buffer size (4097 bytes on Unix, 16384 on Windows),
and `E = 3` on Windows or `1` on Unix. Three encoded bytes per UTF-16 unit cover
WTF-8 conversion; no Windows byte/unit equality is assumed.

```text
U = P + E * S * T
C = N + S * (T + 1) + 1
W = 32 * U + 16 * C + (S + 1) * (B + 16) + R + 16
```

`C` covers all initial/expanded components and special trailing-dot handling.
Thirty-two scan/copy passes per encoded unit leave eight for retained input,
C-string and initial path checks/decomposition, twelve for target record
validation/conversion, trailing-path checks and worklist construction, and
twelve for per-component name checks, native-path rebuilding/encoding and owned
copies. These are abstract payload passes, not counts of machine instructions
or allocator copies; initial and expanded text use the same conservative weight.

Sixteen actions per component cover dispatch, worklist/parent-handle updates,
native open, error classification, dot-access checks and handle cleanup. The
Linux error branch can issue `openat`, initial `statx`, a support-check `statx`,
then `statat`; Windows can open and inspect a component's handle. These branches
are finite and fit that allowance. A fixed link buffer is initialized for each
read; at most `S` reads succeed and one more can fail. Its sixteen extra actions
cover allocation/conversion setup, validation, Unix read or Windows open/query,
and cleanup. The final sixteen cover trailing-dot/borrowed-root open and the
wrapper's same-handle regular-file metadata inspection and result cleanup.
All formula arithmetic is checked; invalid sizes and overflow return `None`.

## Guarantee limits

Each native API invocation is one abstract action, including a Linux fast-path
open that resolves links inside the OS. Up to four such actions plus a complete
manual fallback are included. The 4096-unit limit governs targets returned to
the manual resolver; it is not an OS-wide symlink policy for fast-path opens.
There are no link-buffer growth retries in the patched manual open path. Errors,
including interruptions or insufficient buffers, are returned without retry.

The envelope does not bound kernel iterations, syscall latency, allocator
internals, RSS or wall time. It adds no cancellation guarantee inside a syscall.
Existing Unix nonblocking options, Windows parent/root share modes, native
target gates, containment traversal and final same-handle inspection are retained.
Other crate operations are not certified by `open_work_bound`; in particular,
Windows' separate public `read_link_impl` still uses its original upstream
implementation. Dependency wiring, session admission, actual accounting and
platform qualification belong to the embedding workspace.
