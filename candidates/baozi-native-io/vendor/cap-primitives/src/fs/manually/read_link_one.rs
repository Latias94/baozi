use crate::fs::{errors, read_link_unchecked, MAX_SYMLINK_EXPANSIONS, MAX_SYMLINK_TARGET_UNITS};
use std::ffi::OsStr;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::{fs, io};

/// This is a wrapper around `read_link_unchecked` which performs a single
/// symlink expansion on a single path component, and which enforces the
/// recursion limit.
pub(super) fn read_link_one(
    base: &fs::File,
    name: &OsStr,
    symlink_count: &mut u8,
    reuse: PathBuf,
) -> io::Result<PathBuf> {
    let name: &Path = name.as_ref();
    assert!(
        name.as_os_str().is_empty() || name.file_name().is_some(),
        "read_link_one expects a single normal path component, got '{}'",
        name.display()
    );
    assert!(
        name.as_os_str().is_empty() || name.parent().unwrap().as_os_str().is_empty(),
        "read_link_one expects a single normal path component, got '{}'",
        name.display()
    );

    if *symlink_count == MAX_SYMLINK_EXPANSIONS {
        return Err(errors::too_many_symlinks());
    }

    let destination = read_link_unchecked(base, name, reuse)?;

    // Check native units before the caller scans or copies target components.
    #[cfg(not(windows))]
    let target_units = destination.as_os_str().len();
    #[cfg(windows)]
    let target_units = destination
        .as_os_str()
        .encode_wide()
        .take(MAX_SYMLINK_TARGET_UNITS + 1)
        .count();
    if target_units > MAX_SYMLINK_TARGET_UNITS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "symlink target exceeds the native-unit limit",
        ));
    }

    *symlink_count += 1;

    Ok(destination)
}
