//! Bounds for Baozi's normalized, read-only native asset opens.

/// Maximum successful manual symlink expansions during one open.
///
/// This preserves upstream's distinct Windows and rustix limits. It does not
/// describe the number of links resolved inside a native syscall.
pub const MAX_SYMLINK_EXPANSIONS: u8 = if cfg!(windows) { 63 } else { 40 };

/// Maximum length of a manually expanded symlink target.
///
/// Units are bytes on Unix and UTF-16 code units on Windows. This limit applies
/// before constructing the target's component worklist. Native fast-path opens
/// resolve links inside the OS without returning targets to this resolver.
pub const MAX_SYMLINK_TARGET_UNITS: usize = 4096;

/// Maximum Linux `openat2` calls before returning or using the manual fallback.
///
/// This is the total number of attempts, not a number of additional retries.
/// Other platforms do not use this fast path.
pub const MAX_OPENAT2_ATTEMPTS: usize = if cfg!(target_os = "linux") { 4 } else { 0 };

#[cfg(windows)]
pub(crate) const LINK_READ_BUFFER_BYTES: usize =
    windows_sys::Win32::Storage::FileSystem::MAXIMUM_REPARSE_DATA_BUFFER_SIZE as usize;
#[cfg(not(windows))]
pub(crate) const LINK_READ_BUFFER_BYTES: usize = MAX_SYMLINK_TARGET_UNITS + 1;

/// Returns a conservative abstract work envelope before a native asset open.
///
/// `path_bytes` is the UTF-8 byte length of the normalized relative input, and
/// `path_components` is its number of nonempty normal components. The caller
/// must exclude prefixes, roots, `.` and `..`, and preserve those exact inputs
/// for the open. The envelope covers a retained input copy, the native fast
/// path plus manual fallback, target scanning/copying, fixed link buffers,
/// component actions, helper calls/cleanup and same-handle metadata inspection.
///
/// It applies to read-only asset options, with no create/truncate/write or
/// custom `O_PATH` request; Unix `O_NONBLOCK` is permitted. Each native API call
/// is one abstract action, regardless of its internal kernel work or duration.
/// This is not a bound on allocator internals, RSS, CPU instructions or elapsed
/// time. `None` means arithmetic overflow, invalid sizes, `racy_asserts`, or a
/// platform whose open path is outside this envelope (including FreeBSD).
/// It does not qualify or register a target for Baozi.
///
/// With `S = MAX_SYMLINK_EXPANSIONS`, `T = MAX_SYMLINK_TARGET_UNITS`,
/// `R = MAX_OPENAT2_ATTEMPTS`, buffer bytes `B`, and encoding factor `E`
/// (3 on Windows, 1 on Unix), the envelope is:
///
/// ```text
/// U = path_bytes + E * S * T
/// C = path_components + S * (T + 1) + 1
/// W = 32 * U + 16 * C + (S + 1) * (B + 16) + R + 16
/// ```
///
/// The weights allow thirty-two scan/copy passes per encoded unit and sixteen
/// dispatch/helper/cleanup actions per component. Link reads have no growth
/// retries; at most `S` succeed and one more can fail. See `PATCHES.md` for the
/// accounting scope and the bounded branches covered by these weights.
pub fn open_work_bound(path_bytes: u64, path_components: u64) -> Option<u64> {
    if cfg!(racy_asserts)
        || !cfg!(any(target_os = "linux", target_os = "macos", windows))
        || path_bytes == 0
        || path_components == 0
        || path_components > path_bytes
    {
        return None;
    }

    let links = u64::from(MAX_SYMLINK_EXPANSIONS);
    let target = MAX_SYMLINK_TARGET_UNITS as u64;
    let expanded = links.checked_mul(target)?;
    let encoding = if cfg!(windows) { 3 } else { 1 };
    let units = path_bytes.checked_add(expanded.checked_mul(encoding)?)?;
    let components = path_components
        .checked_add(links.checked_mul(target.checked_add(1)?)?)?
        .checked_add(1)?;
    let link_work = links
        .checked_add(1)?
        .checked_mul((LINK_READ_BUFFER_BYTES as u64).checked_add(16)?)?;

    units
        .checked_mul(32)?
        .checked_add(components.checked_mul(16)?)?
        .checked_add(link_work)?
        .checked_add(MAX_OPENAT2_ATTEMPTS as u64)?
        .checked_add(16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_sizes_and_overflow_have_no_envelope() {
        assert_eq!(open_work_bound(0, 0), None);
        assert_eq!(open_work_bound(1, 0), None);
        assert_eq!(open_work_bound(1, 2), None);
        assert_eq!(open_work_bound(u64::MAX, 1), None);
        assert_eq!(open_work_bound(u64::MAX, u64::MAX), None);
        assert_eq!(open_work_bound(u64::MAX / 32, 1), None);
    }

    #[test]
    fn platform_envelopes_include_targets_buffers_and_fast_fallback() {
        let expected = if cfg!(racy_asserts) {
            None
        } else if cfg!(windows) {
            Some(29_952_064)
        } else if cfg!(target_os = "linux") {
            Some(8_033_677)
        } else if cfg!(target_os = "macos") {
            Some(8_033_673)
        } else {
            None
        };
        assert_eq!(open_work_bound(1, 1), expected);
    }

    #[test]
    fn larger_input_adds_text_and_component_work() {
        if let Some(base) = open_work_bound(1, 1) {
            assert_eq!(open_work_bound(2, 1), Some(base + 32));
            assert_eq!(open_work_bound(3, 2), Some(base + 80));
        }
    }
}
