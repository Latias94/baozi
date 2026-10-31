use crate::fs::MAX_SYMLINK_TARGET_UNITS;
use rustix::fs::readlinkat_raw;
use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
#[cfg(target_os = "wasi")]
use std::os::wasi::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::{fs, io};

/// *Unsandboxed* function similar to `read_link`, but which does not perform
/// sandboxing.
pub(crate) fn read_link_unchecked(
    start: &fs::File,
    path: &Path,
    reuse: PathBuf,
) -> io::Result<PathBuf> {
    // One sentinel byte distinguishes an exact-limit target from a truncated
    // one without the allocation-growth/retry loop in rustix::readlinkat.
    let mut buffer = [0_u8; MAX_SYMLINK_TARGET_UNITS + 1];
    let length = readlinkat_raw(start, path, &mut buffer[..])?;
    if length > MAX_SYMLINK_TARGET_UNITS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "symlink target exceeds the native-unit limit",
        ));
    }
    let target = &buffer[..length];
    if target.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "symlink target contains a NUL byte",
        ));
    }
    let mut bytes = reuse.into_os_string().into_vec();
    bytes.clear();
    bytes.extend_from_slice(target);
    Ok(OsString::from_vec(bytes).into())
}
