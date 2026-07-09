# Parser Threat Model

Baozi parsers process untrusted input. This document summarizes the threat model that informs ADR 0010 and ADR 0014.

## Assets at Risk

- host process stability
- memory and CPU budget
- filesystem boundaries
- asset pipeline correctness
- downstream renderer correctness
- build and CI reliability

## Input Surfaces

- primary model files
- sidecar material and texture files
- archive entries
- embedded binary blobs
- data URIs
- text encodings
- third-party parser backends
- optional FFI backends

## Threats

| Threat | Example | Required defense |
| --- | --- | --- |
| Path traversal | MTL references `../../secret` | root-scoped `AssetIo` |
| Archive escape | zip entry with absolute path | archive path normalization |
| Decompression bomb | tiny archive expands to huge data | decompressed byte limits |
| Count overflow | vertex count overflows allocation size | checked arithmetic |
| Out-of-bounds offset | binary chunk points outside buffer | bounds validation |
| Recursive include | self-referencing sidecar | recursion depth limit |
| Invalid encoding | malformed UTF-8 in required text | structured parse error |
| Panic on malformed input | parser uses unchecked indexing | parser tests and fuzzing |
| OOM | declared count causes huge allocation | resource limits before allocation |
| FFI crash | native decoder aborts | opt-in isolated backend |

## Required Practices

- Use `ResourceLimits` for all parser entry points.
- Return `BaoziError` and diagnostics instead of panicking.
- Add malformed fixtures for every stable parser.
- Add fuzz targets before stable promotion.
- Keep experimental parser slices, such as STL, covered by malformed tests and a compileable fuzz
  target even before stable promotion.
- Keep FFI out of default features.
- Keep remote fetching outside core Baozi.

## Review Checklist

- Are all declared counts validated before allocation?
- Are offsets checked before slicing?
- Can sidecars escape the asset scope?
- Are archive entries normalized before access?
- Can recursion or include depth become unbounded?
- Are all third-party parser errors converted into Baozi errors?
- Does malformed input have regression tests?
- Is every `unsafe` block justified and covered?
