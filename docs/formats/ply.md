# PLY Format Support

## Summary

- Format: PLY
- Crate: `baozi-format-ply`
- Declaration evidence: Unverified
- Baozi maturity: None (no current `BaoziVerified` evidence record)
- Default feature: not enabled by `default-formats`
- Parser backend: Baozi-owned parser
- Supported extensions: `.ply`
- Supported media types: `model/ply`
- Encoding: ASCII, binary little-endian, binary big-endian
- Sidecar policy: none

## Current Status

`baozi-format-ply` imports common vertex and face geometry into Baozi's owned IR. It is marked
`publish = false` while fixture coverage, fuzzing, and managed format evidence are still growing.

## Supported MVP

- ASCII, binary little-endian, and binary big-endian PLY.
- Vertex positions from `x`, `y`, `z`.
- Optional normals from `nx`, `ny`, `nz`.
- Optional vertex colors from `red/green/blue/alpha` or `r/g/b/a`.
- Optional texture coordinates from `s/t`, `u/v`, or `texture_u/texture_v`.
- Point clouds when no face element is present.
- Triangle faces and polygon faces from `vertex_indices` or `vertex_index` list properties.
- Unknown scalar vertex properties preserved as `ply:<name>` custom vertex attributes when they map
  to Baozi's current typed attribute model.
- Comments and `obj_info` lines preserved as mesh metadata.
- Resource-limit checks for primary and decoded UTF-8 bytes, header line/token/string sizes, numeric
  literals, vertex count, custom vertex attribute stream/cell counts, face count, list lengths, mesh
  count, diagnostics, and cooperative cancellation checkpoints.

## Text Surface, Tokenization, and Failure Offsets

- ASCII PLY counts the complete input against `max_decoded_utf8_bytes`. Binary little-endian and
  big-endian PLY count only the textual header through `end_header`; the binary body is neither
  UTF-8-profiled nor charged as decoded text.
- Primary bytes remain borrowed. Each header or ASCII-body line is incrementally validated and copied
  into an owned, live-reserved `ProfiledAsciiLine` in at-most-4096-byte scan quanta. LF, CRLF, and
  horizontal tab are admitted; BOM, bare CR, non-ASCII, and other forbidden controls fail at their
  first raw input byte.
- Token boundary discovery is one forward byte pass over each profiled line. It polls independently
  every 4096 bytes, charges each token once, and reserves token-reference storage before insertion; it
  does not perform an unmetered split/count prepass.
- ASCII scalar literals and header element counts have a format-owned maximum of 4096 bytes. A
  4096-byte literal reaches the typed numeric parser; a 4097-byte literal is rejected before that
  parser is entered. Numeric syntax is locale-independent and uses a dot decimal separator.
- The retained outer `PlyFailure.byte_offset` and packed `byte_offset` detail are source-global raw
  offsets computed from the line base plus the offending chunk-relative byte. When invalid UTF-8 can
  provide a standard source, `FormatFailureEnvelope` retains `PlyFailure`, its private PLY source
  wrapper, and `std::str::Utf8Error` as successive source-chain hops. `Utf8Error::valid_up_to()` remains
  relative to the validation chunk and must not be interpreted as the source-global offset.

The profile, decoded-surface, numeric-boundary, global-offset, and source-chain contracts are covered
by `crates/baozi-format-ply/tests/text_profile.rs`.

## Cancellation and Collection Checkpoints

- Raw line discovery, ASCII profile validation, tokenization, and owned string copies each poll at
  their own at-most-4096-byte boundary. String copies preserve UTF-8 character boundaries.
- Header element/property loops, body element/record/property loops, scene-construction collections,
  and other potentially large collection passes poll at least once per 4096 items through the shared
  collection checkpoint.
- Work permits keep attempted and consumed evidence distinct. A record admitted before a profile,
  token, or numeric failure remains attempted but not consumed.

Long-string copy cancellation is covered by `crates/baozi-format-ply/tests/limits.rs`; collection,
profile, and failed-record accounting are exercised by the format integration suites.

## Known Non-Support

- Materials, textures, cameras, lights, skinning, morph targets, and animation.
- Non-scalar custom properties and non-geometry elements are skipped with diagnostics.
- Coordinate system and unit metadata are not standardized by the format and remain `Unknown`.
- The parser has no Baozi maturity until managed fuzz and broader fixture evidence are available.
