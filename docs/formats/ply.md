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
- Primary bytes remain borrowed. The import-owned `TextInputRecordCursor` discovers one exact
  source-bound physical record at a time. Reject-invalid ASCII framing stops at the first byte over
  `max_line_bytes`; otherwise `ConformedTextInput` performs the shared metered decode/newline/span
  plan and build. LF, CRLF, and horizontal tab are admitted; BOM, bare CR, non-ASCII, invalid UTF-8,
  and other forbidden controls fail at their first raw input byte.
- Header parsing stops immediately after the exact `end_header` record range. Its exclusive raw end
  becomes `data_start`, so a binary payload beginning with LF, CR, space, NUL, or a high byte remains
  byte-for-byte owned by the binary cursor. ASCII parsing continues the same text cursor only for the
  declared records; trailing bytes retain the format's existing ignored-tail policy.
- Decoded record storage and compact line indexes are live-reserved under the shared
  `DecodedTextBytes` and `LineIndexBytes` dimensions. Because records are processed incrementally,
  their peak text storage is bounded by the active line rather than the complete ASCII asset.
- Token boundary discovery is one forward byte pass over each conformed line. It polls independently
  every 4096 bytes, charges each token once, and reserves token-reference storage before insertion; it
  does not perform an unmetered split/count prepass.
- ASCII scalar literals and header element counts have a format-owned maximum of 4096 bytes. A
  4096-byte literal reaches the typed numeric parser; a 4097-byte literal is rejected before that
  parser is entered. Numeric syntax is locale-independent and uses a dot decimal separator.
- The format-private `PlyFailure.byte_offset` and packed `byte_offset` detail are source-global raw
  offsets taken from shared line and violation ranges. Format grammar errors remain anchored at the
  physical line start, while text-profile failures use the exact offending byte.
- `ParseScope` copies the stable code plus bounded `reason`, `context`, and `byte_offset` details into
  Baozi-owned `FormatFailureEvidence`, then destroys `PlyFailure` and any transient parser error. The
  public error chain terminates at that host-owned evidence and exposes no PLY/backend downcast.

The profile, decoded-surface, numeric-boundary, global-offset, packed-evidence, and teardown contracts
are covered by `crates/baozi-format-ply/tests/text_profile.rs` and the import session contract tests.

## Cancellation and Collection Checkpoints

- Shared record-boundary discovery polls at an at-most-4096-byte boundary. An ASCII line overrun
  publishes only the bounded prefix through `max_line_bytes + 1`; each exact record otherwise
  performs two metered decode passes for planning and construction. Tokenization and format-owned
  string copies retain their own at-most-4096-byte checkpoints.
- Header element/property loops, body element/record/property loops, scene-construction collections,
  and other potentially large collection passes poll at least once per 4096 items through the shared
  collection checkpoint.
- Explicit parent work keeps attempted and consumed evidence distinct while shared decode and
  format-owned token work run as children. A record admitted before profile, allocation,
  cancellation, token, or numeric failure remains attempted but not consumed.

Long-string copy cancellation is covered by `crates/baozi-format-ply/tests/limits.rs`; collection,
profile, and failed-record accounting are exercised by the format integration suites.

## Known Non-Support

- Materials, textures, cameras, lights, skinning, morph targets, and animation.
- Non-scalar custom properties and non-geometry elements are skipped with diagnostics.
- Coordinate system and unit metadata are not standardized by the format and remain `Unknown`.
- The parser has no Baozi maturity until managed fuzz and broader fixture evidence are available.
