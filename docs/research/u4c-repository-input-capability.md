# U4c Repository Input Capability: Public Rust API Research

Status: research recommendation, 2026-07-26

## Question and decision

U4c needs repository evidence gathered without path re-opening: traversal must remain
relative to an already-open root, each path component must be opened without following
links, directory contents must be enumerated through directory handles, and object
identity and content must come from the same open handle.

**Decision:** the exact public combination of `cap-std 4.0.2` and
`cap-fs-ext 4.0.2` is sufficient for the baseline U4c repository walker on the
admitted Unix filesystems. It also supplies Windows no-follow and same-handle
reparse checks, but not a safely qualified full-width file identity. It provides a
smaller and more portable trusted surface than a Baozi-owned syscall layer. It does
not provide one call that secures a multi-component path, an atomic directory
snapshot, or a full-width ReFS file identifier. Those limitations require explicit
composition, and Windows capture must remain fail closed until its identity gap is
separately admitted.

The current workspace pins `cap-std = "=4.0.2"` and `rustix = "=1.1.4"`.
As of the research date, these are also the newest non-yanked releases, and the
matching newest `cap-fs-ext` release is `4.0.2`:

| Crate | Workspace state | Current stable | Relevant feature boundary |
| --- | --- | --- | --- |
| [`cap-std`](https://crates.io/api/v1/crates/cap-std) | Exact `4.0.2` | `4.0.2` | Default features are empty. |
| [`cap-fs-ext`](https://crates.io/api/v1/crates/cap-fs-ext) | Exact `4.0.2` on the repository candidate | `4.0.2` | `std` enables implementations for `cap_std` types. |
| [`rustix`](https://crates.io/api/v1/crates/rustix) | Exact `1.1.4` | `1.1.4` | `fs` is opt-in and the filesystem module is not available on Windows. |

`cap-fs-ext 4.0.2` declares compatible `cap-std ^4.0.2` and
`cap-primitives ^4.0.2` dependencies
([manifest](https://github.com/bytecodealliance/cap-std/blob/715e4ed607ae9a93c7446b0fa63296f7898831c2/cap-fs-ext/Cargo.toml)).
Using the exact public crate versions below plus the committed lockfile keeps the
reviewed release set stable:

```toml
cap-std = { version = "=4.0.2", default-features = false }
cap-fs-ext = { version = "=4.0.2", default-features = false, features = ["std"] }
```

Enabling `cap-fs-ext/std` is required when default features are disabled; otherwise
the extension implementations for `cap_std::fs::Dir` and its metadata types are not
available. `fs_utf8` is unnecessary for the repository walker. The admitted portable
profile deliberately accepts only ASCII components, preserves their spelling, and
fails closed on Unicode rather than silently choosing a normalization or case-folding
contract that is not consistent across the admitted filesystems.

## Requirement-to-API mapping

| U4c requirement | Public API | Assessment |
| --- | --- | --- |
| Capability-relative root | [`cap_std::fs::Dir`](https://docs.rs/cap-std/4.0.2/cap_std/fs/struct.Dir.html) | Satisfied. A `Dir` retains an open directory capability and its operations are relative to it. |
| Per-component directory nofollow | [`cap_fs_ext::DirExt::open_dir_nofollow`](https://docs.rs/cap-fs-ext/4.0.2/cap_fs_ext/trait.DirExt.html#tymethod.open_dir_nofollow) | Satisfied by passing exactly one normal component per call and retaining each returned `Dir`. |
| Final-file nofollow | [`OpenOptionsFollowExt::follow(FollowSymlinks::No)`](https://docs.rs/cap-fs-ext/4.0.2/cap_fs_ext/trait.OpenOptionsFollowExt.html) | Satisfied only when the supplied path is one component. The option governs the last component, not every component in a compound path. |
| Directory enumeration | [`Dir::entries`](https://docs.rs/cap-std/4.0.2/cap_std/fs/struct.Dir.html#method.entries) and [`DirEntry`](https://docs.rs/cap-std/4.0.2/cap_std/fs/struct.DirEntry.html) | Satisfied. `DirEntry` deliberately exposes a name rather than an ambient path. Enumeration is not an atomic snapshot. |
| Metadata from the open object | [`File::metadata`](https://docs.rs/cap-std/4.0.2/cap_std/fs/struct.File.html#method.metadata) and [`Dir::dir_metadata`](https://docs.rs/cap-std/4.0.2/cap_std/fs/struct.Dir.html#method.dir_metadata) | Satisfied. Both query the retained file or directory handle, so no authorization-sensitive path lookup is needed. |
| Handle identity | [`cap_fs_ext::MetadataExt::{dev, ino, nlink}`](https://docs.rs/cap-fs-ext/4.0.2/cap_fs_ext/trait.MetadataExt.html) | Satisfied for the admitted Unix baseline. On Windows, `dev` and `ino` map to a 64-bit file index that cannot qualify ReFS, and the reviewed safe API cannot identify the filesystem from the same handle. |
| Windows reparse rejection | Public `cap_fs_ext::OsMetadataExt::file_attributes()` re-export ([source](https://github.com/bytecodealliance/cap-std/blob/715e4ed607ae9a93c7446b0fa63296f7898831c2/cap-fs-ext/src/lib.rs)) | Satisfied as a Baozi policy check on the just-opened handle. Reject any `FILE_ATTRIBUTE_REPARSE_POINT`, not only documented symlink forms. |
| Nonblocking open policy | [`cap_fs_ext::OpenOptionsSyncExt::nonblock`](https://docs.rs/cap-fs-ext/4.0.2/cap_fs_ext/trait.OpenOptionsSyncExt.html) | Available through a public API. U4c need not reach into raw `rustix::OFlags` solely for this setting. |

## Correct composition

There is intentionally no public API that means "open this arbitrary relative path
and reject links in every component." `FollowSymlinks::No` applies to the final
component ([upstream definition](https://github.com/bytecodealliance/cap-std/blob/715e4ed607ae9a93c7446b0fa63296f7898831c2/cap-primitives/src/fs/follow_symlinks.rs)).
U4c must therefore make the component boundary structural:

1. Accept only repository-relative `Component::Normal` names. Reject prefixes,
   roots, `.` and `..` before opening anything.
2. Enumerate the current `Dir`, validate each returned native name, and sort the
   evidence representation deterministically.
3. For a directory name, call `open_dir_nofollow` with that one name, validate
   metadata from the returned `Dir`, then use it as the capability for the next
   level.
4. For a file name, call `Dir::open_with` with that one name and options containing
   `follow(FollowSymlinks::No)`. Classify, identify, and read through the returned
   `File`.
5. Never use path metadata, canonicalization, an existence probe, or a second open
   to authorize the object already represented by the handle.

`DirEntry::file_type()` may be used only as a routing hint for the directory-open or
file-open operation. It is not authorization metadata. If the type observed from
the successfully opened handle differs from the hint or from the expected open
branch, invalidate the observation pass rather than re-opening the name to obtain a
different kind of handle. Likewise, only a normalized type-mismatch result may
select the alternate open branch; permission, I/O, and unsupported-object errors
must remain evidence failures.

This construction preserves the binding required by
[ADR 0010](../adr/0010-asset-io-virtual-filesystem-uri-archive-and-path-security.md):
the successful open chooses the object, while identity, classification, size and
bytes are observations of that object.

On Windows, nofollow must not be interpreted as "all reparse points are harmless."
After each successful directory or file open, inspect `file_attributes()` on that
same handle and fail closed when `FILE_ATTRIBUTE_REPARSE_POINT` is present. A named
`windows-sys` constant is preferable to a copied numeric value. This is a small
platform policy branch, not a custom Windows open or identity implementation.

## Identity semantics and Windows qualification

`cap_fs_ext::MetadataExt` is the intended public boundary. Its Windows implementation
maps `dev()` to `volume_serial_number` and `ino()` to `file_index`
([implementation](https://github.com/bytecodealliance/cap-std/blob/715e4ed607ae9a93c7446b0fa63296f7898831c2/cap-fs-ext/src/metadata_ext.rs)).
The dependency's hidden `_WindowsByHandle` trait explicitly directs consumers to
`cap_fs_ext::MetadataExt`
([source](https://github.com/bytecodealliance/cap-std/blob/715e4ed607ae9a93c7446b0fa63296f7898831c2/cap-primitives/src/fs/metadata.rs)).
Likewise, the hidden `_cap_fs_ext_follow` hook explicitly directs consumers to
`OpenOptionsFollowExt`
([source](https://github.com/bytecodealliance/cap-std/blob/715e4ed607ae9a93c7446b0fa63296f7898831c2/cap-primitives/src/fs/open_options.rs)).
Direct use of either hidden API is therefore both unnecessary and contrary to the
upstream ownership boundary.

Rust's stable Windows
[`std::os::windows::fs::MetadataExt`](https://doc.rust-lang.org/stable/std/os/windows/fs/trait.MetadataExt.html)
does not close this gap: `volume_serial_number`, `file_index`, and
`number_of_links` remain gated by the unstable `windows_by_handle` feature. On Unix,
the corresponding stable standard trait exposes `dev` and `ino`
([documentation](https://doc.rust-lang.org/stable/std/os/unix/fs/trait.MetadataExt.html)).
The extension crate is consequently material for a stable, common implementation,
especially on Windows.

The public `ino()` contract is only 64 bits. Upstream records that ReFS file
identifiers are 128 bits and that returning 64 bits is incomplete
([source FIXME](https://github.com/bytecodealliance/cap-std/blob/715e4ed607ae9a93c7446b0fa63296f7898831c2/cap-fs-ext/src/metadata_ext.rs)).
Baozi must choose one of two explicit policies:

- Baseline decision: admit Unix filesystems for which `(dev, ino)` is the
  documented handle identity. Fail closed on Windows because the reviewed safe API
  cannot establish whether the 64-bit identity contract is sufficient for the
  opened filesystem.
- Broader Windows support: add a narrowly owned Windows platform adapter that
  queries the full 128-bit `FILE_ID_INFO` from the already-open handle. This is the
  only identified reason U4c needs a custom identity layer.

Identity is a traversal-session alias and drift key, not content identity or
permanent evidence. Evidence must also include the content digest and the validated
repository-relative name. Reuse of an inode or file index after a handle closes must
not be treated as proof that content is unchanged.

### Safe Windows identity crate audit

No reviewed crate simultaneously provides a mature, license-compatible, safe API
over an already-open handle for full-width identity, reparse attributes, and
filesystem qualification:

| Candidate | Blocking fact |
| --- | --- |
| [`fs-id 0.2.0`](https://github.com/Rosy-iso/fs-id/blob/v0.2.0/src/windows.rs) | It is the only reviewed safe API returning the complete 128-bit `FILE_ID_INFO` from `AsRawHandle`, but it is minimally adopted, does not contractually support directories or filesystem/reparse qualification, and uses Zlib outside Baozi's admitted license set. |
| [`file-id 0.2.3`](https://docs.rs/file-id/0.2.3/src/file_id/lib.rs.html) | Its public API accepts a path and reopens it; the convenience path may also fall back from high- to low-resolution identity. |
| `winapi-util` / `same-file` | They use the older 64-bit `BY_HANDLE_FILE_INFORMATION` identity. |
| `windows` / `windows-sys` | They expose the required Win32 calls, but invoking them requires Baozi-owned `unsafe`. |

Microsoft specifies the full comparison key as volume serial plus the 128-bit ID in
[`FILE_ID_INFO`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_info),
and explicitly warns that the older 64-bit file index is not guaranteed unique on
ReFS in
[`BY_HANDLE_FILE_INFORMATION`](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/ns-fileapi-by_handle_file_information).
`GetVolumeInformationByHandleW` can report the filesystem from a handle, but the
Rust bindings expose that call through an unsafe FFI boundary and the operation is
not available for every remote filesystem. Consequently, accepting a caller's
filesystem assertion or inspecting only the low 64 bits would not be fail closed.

## Why not use rustix as the repository abstraction

`rustix 1.1.4` is valuable as a low-level Unix building block:

- [`openat`](https://docs.rs/rustix/1.1.4/rustix/fs/fn.openat.html) plus
  [`OFlags::NOFOLLOW`](https://docs.rs/rustix/1.1.4/rustix/fs/struct.OFlags.html)
  can implement a conservative component loop.
- Linux
  [`openat2`](https://docs.rs/rustix/1.1.4/rustix/fs/fn.openat2.html) supports
  [`ResolveFlags`](https://docs.rs/rustix/1.1.4/rustix/fs/struct.ResolveFlags.html)
  such as `BENEATH`, `NO_MAGICLINKS`, and `NO_SYMLINKS`.
- [`fstat`](https://docs.rs/rustix/1.1.4/rustix/fs/fn.fstat.html) returns identity
  from an already-open descriptor.
- [`Dir`](https://docs.rs/rustix/1.1.4/rustix/fs/struct.Dir.html) and
  [`RawDir`](https://docs.rs/rustix/1.1.4/rustix/fs/struct.RawDir.html) enumerate
  directory descriptors.

It is not a cross-platform repository capability abstraction. The `rustix::fs`
module is excluded on Windows
([crate source](https://github.com/bytecodealliance/rustix/blob/v1.1.4/src/lib.rs)),
and `openat` nofollow still covers only its final component. A rustix-first design
would make Baozi own Unix component parsing, Linux `openat2` capability detection and
fail-closed fallback, directory iteration details, plus a separate Windows
`NtCreateFile` and handle-identity backend. That duplicates mature logic already
owned by the cap-std project and substantially enlarges Baozi's platform-specific
trusted code.

Therefore:

- retain direct `rustix` only where ADR 0010 already admits a narrowly named Unix
  mechanism;
- do not use it as U4c's portable repository API;
- do not silently weaken Linux policy when `openat2` returns `NOSYS`;
- prefer the public cap crates for the U4c walker.

## Ownership and admission recommendation

The handle-native mechanism should be private to `baozi-io`: root adoption,
single-component nofollow opens, same-handle metadata, same-handle reads, and
platform error normalization belong next to the production filesystem boundary.
Do not expose `cap_std`, `cap_fs_ext`, raw descriptors, or raw handles in Baozi's
public API.

`baozi-xtask` should own U4c evidence policy: selected roots, concrete limit values,
text classification, content digests, qualification thresholds, and the
human-readable conformance report. `baozi-io` should validate the strict portable
path type, enforce the caller's limits, reject unsupported object kinds and identity
aliases, produce deterministic captures, and compare a complete recapture against
the opaque baseline. Keeping identity and exact closure comparison inside IO avoids
leaking platform identity into xtask while still leaving evidence meaning with the
tool.

Admit the new dependency behind a distinct candidate/internal
`repository-fs-candidate` feature until platform tests and the dependency closure
have been accepted. It must not be folded into, or presented as conformance of, the
existing runtime `fs-candidate` feature. Remove direct U4c use of `cap-primitives`;
it is an implementation crate, not the public extension surface. U4c also does not
need a direct rustix dependency merely to set `NONBLOCK` when
`OpenOptionsSyncExt::nonblock(true)` is available.

There is an ADR constraint: the currently accepted native adapter text in ADR 0010
explicitly states that it uses no additional filesystem extension crate. Adding
`cap-fs-ext` to the ADR-governed runtime path therefore requires an accepted ADR
amendment and dependency-admission update. Research or candidate U4c use is not
evidence that the existing runtime feature already conforms, and must not silently
change that claim.

## Residual risks and required fail-closed behavior

1. **Enumeration is not a snapshot.** No reviewed API makes a live directory tree
   atomic. Capture names, handle identities and hashes, then re-enumerate and compare
   the complete closure. This detects stable drift but not a transient A-to-B-to-A
   change. Endpoint recapture proves only that the compared endpoint observations
   agree; it does not prove that the repository remained continuously unchanged
   between them. Strict evidence should run against an immutable materialized
   snapshot or a repository held quiescent by a trusted producer.
2. **Windows filesystem identity is unavailable to the admitted safe boundary.**
   Compile and dependency-audit the candidate, but reject capture before traversal.
   Enabling Windows requires the safe full-width adapter described above; a 64-bit
   fallback or caller assertion is not qualification.
3. **All Windows reparse points need policy rejection.** Test symlinks, junctions
   and representative non-symlink reparse tags. Do not infer an all-reparse guarantee
   from an API documented in terms of symlinks.
4. **Metadata errors are evidence failures.** `File::metadata` or `Dir::dir_metadata`
   failure must abort that repository observation; do not fall back to path metadata.
5. **Names remain Baozi policy.** The baseline admits only ASCII. Case-folding aliases,
   Windows short names, Unicode or non-Unicode native names, reserved components and
   cross-platform spelling collisions are not solved by capability APIs. Expanding the
   profile requires a separate normalization and collision decision.
6. **Resource bounds remain Baozi policy.** Apply deterministic limits to depth,
   entry count, total bytes, individual file size and diagnostic volume.
7. **Object kinds must be allowlisted.** Reject links, reparse points, devices,
   sockets, FIFOs and other unsupported kinds from same-handle metadata before
   reading.
8. **Dependency closure changes require review.** Exact direct versions and the
   committed lockfile define the researched baseline. Future lockfile updates must
   re-run platform conformance tests even if semver accepts the transitive release.

## Recommended conclusion for U4c

Use `cap-std = "=4.0.2"` plus
`cap-fs-ext = { version = "=4.0.2", default-features = false, features = ["std"] }`
as the public Unix repository-handle baseline and the Windows no-follow/reparse
building block. Implement nofollow as an explicit
one-component-at-a-time state machine, derive identity and bytes from each retained
handle, and make Windows capture fail before traversal until a safe same-handle
full-width identity and filesystem qualification are admitted. Keep rustix out of
the portable U4c abstraction and add custom platform code only through a separate
unsafe/dependency decision.

This conclusion is an API and source review. No build, test, `rustc`, `rustdoc`, or
Cargo command was used to establish it.
