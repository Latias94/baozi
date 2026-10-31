#![allow(unsafe_code)]

use crate::fs::{
    open_unchecked, FollowSymlinks, OpenOptions, OpenOptionsExt, LINK_READ_BUFFER_BYTES,
    MAX_SYMLINK_TARGET_UNITS,
};
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Component, Path, PathBuf};
use std::{fs, io};
use windows_sys::Wdk::Storage::FileSystem::SYMLINK_FLAG_RELATIVE;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
};
use windows_sys::Win32::System::Ioctl::FSCTL_GET_REPARSE_POINT;
use windows_sys::Win32::System::SystemServices::{
    IO_REPARSE_TAG_MOUNT_POINT, IO_REPARSE_TAG_SYMLINK,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

/// Read one reparse target relative to its retained parent, without following it.
pub(crate) fn read_link_unchecked(
    start: &fs::File,
    path: &Path,
    _reuse: PathBuf,
) -> io::Result<PathBuf> {
    // Keep open_unchecked on its single-component, relative NtCreateFile path;
    // it must never re-root through a prefix, parent component or ambient path.
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "reparse read requires one relative normal component",
        ));
    }

    // Match the former std::fs::read_link access and sharing options. The parent
    // retains its existing no-FILE_SHARE_DELETE capability handle throughout.
    let mut options = OpenOptions::new();
    options.access_mode(0);
    options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS);
    options.follow(FollowSymlinks::No);
    let file = open_unchecked(start, path, &options)?;

    let mut buffer = [0_u8; LINK_READ_BUFFER_BYTES];
    let mut returned = 0_u32;
    // SAFETY: file remains live for the synchronous call. The output is a valid
    // writable byte buffer of the advertised size, returned is writable, and
    // null input/OVERLAPPED pointers request a synchronous query with no input.
    // No reference to a native reparse struct is formed; all returned fields
    // and ranges are validated below before decoding UTF-16.
    let result = unsafe {
        DeviceIoControl(
            file.as_raw_handle().cast(),
            FSCTL_GET_REPARSE_POINT,
            std::ptr::null(),
            0,
            buffer.as_mut_ptr().cast(),
            buffer.len() as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        return Err(io::Error::last_os_error());
    }
    let bytes = buffer
        .get(..returned as usize)
        .ok_or_else(invalid_reparse)?;
    decode_reparse_target(bytes)
}

fn decode_reparse_target(bytes: &[u8]) -> io::Result<PathBuf> {
    // REPARSE_DATA_BUFFER has an eight-byte common header. ReparseDataLength
    // excludes that header and bounds both the fixed fields and PathBuffer.
    let header = bytes.get(..8).ok_or_else(invalid_reparse)?;
    let tag = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let length = usize::from(u16::from_le_bytes([header[4], header[5]]));
    let end = 8_usize.checked_add(length).ok_or_else(invalid_reparse)?;
    let payload = bytes.get(8..end).ok_or_else(invalid_reparse)?;
    let (fixed, relative) = match tag {
        IO_REPARSE_TAG_SYMLINK => {
            let fields = payload.get(..12).ok_or_else(invalid_reparse)?;
            let flags = u32::from_le_bytes([fields[8], fields[9], fields[10], fields[11]]);
            if flags & !SYMLINK_FLAG_RELATIVE != 0 {
                return Err(invalid_reparse());
            }
            (12, flags & SYMLINK_FLAG_RELATIVE != 0)
        }
        IO_REPARSE_TAG_MOUNT_POINT => {
            payload.get(..8).ok_or_else(invalid_reparse)?;
            (8, false)
        }
        _ => return Err(invalid_reparse()),
    };
    // Both variants have these four u16 fields before any flags. Offsets are
    // relative to PathBuffer, and offsets and lengths are measured in bytes.
    let substitute_offset = usize::from(u16::from_le_bytes([payload[0], payload[1]]));
    let substitute_length = usize::from(u16::from_le_bytes([payload[2], payload[3]]));
    let print_offset = usize::from(u16::from_le_bytes([payload[4], payload[5]]));
    let print_length = usize::from(u16::from_le_bytes([payload[6], payload[7]]));
    let path_buffer = &payload[fixed..];
    if path_buffer.len() % 2 != 0 {
        return Err(invalid_reparse());
    }
    let substitute = utf16_range(path_buffer, substitute_offset, substitute_length)?;
    // Validate even the unused print name; never trust a malformed native record.
    utf16_range(path_buffer, print_offset, print_length)?;
    if substitute.len() / 2 > MAX_SYMLINK_TARGET_UNITS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "symlink target exceeds the native-unit limit",
        ));
    }
    let mut wide: Vec<u16> = substitute
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    if wide.contains(&0) {
        return Err(invalid_reparse());
    }
    // Preserve absolute targets as absolute without std's path conversion and
    // resizing helpers. The manual resolver will reject their prefix/root.
    if !relative && wide.starts_with(&[92, 63, 63, 92]) {
        wide[1] = 92;
    }
    let destination = PathBuf::from(OsString::from_wide(&wide));
    if !relative && !destination.has_root() {
        return Err(invalid_reparse());
    }
    Ok(destination)
}

fn utf16_range(bytes: &[u8], offset: usize, length: usize) -> io::Result<&[u8]> {
    if offset % 2 != 0 || length % 2 != 0 {
        return Err(invalid_reparse());
    }
    let end = offset.checked_add(length).ok_or_else(invalid_reparse)?;
    bytes.get(offset..end).ok_or_else(invalid_reparse)
}

fn invalid_reparse() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid or unsupported reparse data",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::ffi::OsStrExt;

    fn record(tag: u32, flags: u32, target: &[u16]) -> Vec<u8> {
        let fixed = if tag == IO_REPARSE_TAG_MOUNT_POINT {
            8
        } else {
            12
        };
        let length = u16::try_from(fixed + target.len() * 2).unwrap();
        let target_length = u16::try_from(target.len() * 2).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.extend_from_slice(&length.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&target_length.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        if fixed == 12 {
            bytes.extend_from_slice(&flags.to_le_bytes());
        }
        for unit in target {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn relative_targets_preserve_native_spelling() {
        for target in ["asset.bin", "dir/../asset.bin", ".", ".."] {
            let wide: Vec<u16> = target.encode_utf16().collect();
            let bytes = record(IO_REPARSE_TAG_SYMLINK, SYMLINK_FLAG_RELATIVE, &wide);
            let path = decode_reparse_target(&bytes).unwrap();
            assert_eq!(path.as_os_str().encode_wide().collect::<Vec<_>>(), wide);
        }
    }

    #[test]
    fn target_limit_counts_utf16_units_including_unpaired_surrogates() {
        let mut target = vec![0xd800; MAX_SYMLINK_TARGET_UNITS];
        let exact = record(IO_REPARSE_TAG_SYMLINK, SYMLINK_FLAG_RELATIVE, &target);
        let path = decode_reparse_target(&exact).unwrap();
        assert_eq!(path.as_os_str().encode_wide().collect::<Vec<_>>(), target);

        target.push(0xd800);
        let excess = record(IO_REPARSE_TAG_SYMLINK, SYMLINK_FLAG_RELATIVE, &target);
        assert_eq!(
            decode_reparse_target(&excess).unwrap_err().kind(),
            io::ErrorKind::InvalidData,
        );
    }

    #[test]
    fn names_are_relative_to_path_buffer_not_the_record_header() {
        let mut bytes = record(
            IO_REPARSE_TAG_SYMLINK,
            SYMLINK_FLAG_RELATIVE,
            &[b'p' as u16, b'p' as u16, b'a' as u16],
        );
        bytes[8..10].copy_from_slice(&4_u16.to_le_bytes());
        bytes[10..12].copy_from_slice(&2_u16.to_le_bytes());
        bytes[14..16].copy_from_slice(&4_u16.to_le_bytes());
        assert_eq!(decode_reparse_target(&bytes).unwrap(), PathBuf::from("a"));
    }

    #[test]
    fn absolute_targets_remain_rooted_for_the_existing_resolver() {
        let target: Vec<u16> = r"\??\C:\outside\asset.bin".encode_utf16().collect();
        for tag in [IO_REPARSE_TAG_SYMLINK, IO_REPARSE_TAG_MOUNT_POINT] {
            let path = decode_reparse_target(&record(tag, 0, &target)).unwrap();
            assert!(path.has_root());
            assert!(matches!(
                path.components().next(),
                Some(Component::Prefix(_))
            ));
        }
        let relative: Vec<u16> = "asset.bin".encode_utf16().collect();
        assert!(decode_reparse_target(&record(IO_REPARSE_TAG_SYMLINK, 0, &relative)).is_err());
    }

    #[test]
    fn truncated_headers_and_payloads_are_rejected() {
        let target: Vec<u16> = "asset.bin".encode_utf16().collect();
        for tag in [IO_REPARSE_TAG_SYMLINK, IO_REPARSE_TAG_MOUNT_POINT] {
            let bytes = record(tag, SYMLINK_FLAG_RELATIVE, &target);
            for end in 0..bytes.len() {
                assert!(decode_reparse_target(&bytes[..end]).is_err());
            }
        }
    }

    #[test]
    fn both_names_require_aligned_in_payload_ranges() {
        let target: Vec<u16> = "asset.bin".encode_utf16().collect();
        let valid = record(IO_REPARSE_TAG_SYMLINK, SYMLINK_FLAG_RELATIVE, &target);
        for field in [8, 10, 12, 14] {
            let mut odd = valid.clone();
            odd[field..field + 2].copy_from_slice(&1_u16.to_le_bytes());
            assert!(decode_reparse_target(&odd).is_err());

            let mut outside = valid.clone();
            outside[field..field + 2].copy_from_slice(&(u16::MAX - 1).to_le_bytes());
            assert!(decode_reparse_target(&outside).is_err());
        }
        assert!(utf16_range(&[], usize::MAX - 1, 2).is_err());
        let mut odd_payload = valid.clone();
        odd_payload.push(0);
        let length = u16::try_from(odd_payload.len() - 8).unwrap();
        odd_payload[4..6].copy_from_slice(&length.to_le_bytes());
        assert!(decode_reparse_target(&odd_payload).is_err());
    }

    #[test]
    fn unsupported_tags_flags_and_nuls_are_rejected() {
        let target = [b'a' as u16];
        assert!(decode_reparse_target(&record(0, SYMLINK_FLAG_RELATIVE, &target)).is_err());
        assert!(decode_reparse_target(&record(IO_REPARSE_TAG_SYMLINK, 2, &target)).is_err());
        assert!(decode_reparse_target(&record(
            IO_REPARSE_TAG_SYMLINK,
            SYMLINK_FLAG_RELATIVE,
            &[b'a' as u16, 0],
        ))
        .is_err());
    }
}
