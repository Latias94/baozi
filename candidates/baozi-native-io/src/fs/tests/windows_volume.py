"""Create or clear one caller-owned, verified volume-GUID reparse fixture."""

import ctypes
from ctypes import wintypes
from pathlib import Path
import struct
import sys

kernel = ctypes.WinDLL("kernel32", use_last_error=True)
create = kernel.CreateFileW
create.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, wintypes.LPVOID,
                   wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
create.restype = wintypes.HANDLE
control = kernel.DeviceIoControl
control.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.LPVOID, wintypes.DWORD,
                    wintypes.LPVOID, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD),
                    wintypes.LPVOID]
control.restype = wintypes.BOOL
close = kernel.CloseHandle
close.argtypes = [wintypes.HANDLE]
close.restype = wintypes.BOOL
volume_path = kernel.GetVolumePathNameW
volume_path.argtypes = [wintypes.LPCWSTR, wintypes.LPWSTR, wintypes.DWORD]
volume_path.restype = wintypes.BOOL
volume_name = kernel.GetVolumeNameForVolumeMountPointW
volume_name.argtypes = [wintypes.LPCWSTR, wintypes.LPWSTR, wintypes.DWORD]
volume_name.restype = wintypes.BOOL

mode, mount_arg, outside_arg = sys.argv[1:]
mount = Path(mount_arg).absolute()
assert mount.name == "volume" and mount.parent.name.startswith("baozi-io-volume-root-")
# Runner temp paths can contain aliases. Resolve the owned parent, never the
# mount leaf: in clear mode the leaf already points at the volume root.
owned_root = mount.parent.resolve(strict=True)
assert owned_root.name.startswith("baozi-io-volume-root-") and owned_root.is_dir()
mount = owned_root / mount.name
assert mode in ("create", "clear")
if mode == "create":
    assert not mount.is_symlink() and mount.is_dir() and not any(mount.iterdir())

handle = create(str(mount), 0xC0000000, 7, None, 3, 0x02200000, None)
if handle == wintypes.HANDLE(-1).value:
    raise ctypes.WinError(ctypes.get_last_error())


def ioctl(code, data=None, output=None):
    returned = wintypes.DWORD()
    if not control(handle, code, data, len(data) if data is not None else 0,
                   output, len(output) if output is not None else 0,
                   ctypes.byref(returned), None):
        raise ctypes.WinError(ctypes.get_last_error())
    return returned.value


def clear():
    deletion = ctypes.create_string_buffer(struct.pack("<IHH", 0xA0000003, 0, 0), 8)
    ioctl(0x000900AC, deletion)


created = False
try:
    if mode == "clear":
        clear()
    else:
        outside = Path(outside_arg).resolve(strict=True)
        assert not outside.is_relative_to(mount.parent)
        drive = ctypes.create_unicode_buffer(32768)
        if not volume_path(str(outside), drive, len(drive)):
            raise ctypes.WinError(ctypes.get_last_error())
        volume = ctypes.create_unicode_buffer(1024)
        if not volume_name(drive.value, volume, len(volume)):
            raise ctypes.WinError(ctypes.get_last_error())
        assert volume.value.startswith("\\\\?\\Volume{") and volume.value.endswith("}\\")
        substitute = ("\\??\\" + volume.value[4:]).encode("utf-16-le")
        printed = volume.value.encode("utf-16-le")
        names = substitute + b"\0\0" + printed + b"\0\0"
        payload = struct.pack("<HHHH", 0, len(substitute), len(substitute) + 2, len(printed)) + names
        record = struct.pack("<IHH", 0xA0000003, len(payload), 0) + payload
        ioctl(0x000900A4, ctypes.create_string_buffer(record, len(record)))
        created = True
        observed = ctypes.create_string_buffer(16384)
        length = ioctl(0x000900A8, output=observed)
        assert observed.raw[:length] == record
        queried = ctypes.create_unicode_buffer(1024)
        if not volume_name(str(mount) + "\\", queried, len(queried)):
            raise ctypes.WinError(ctypes.get_last_error())
        assert queried.value.casefold() == volume.value.casefold()
        relative = outside.relative_to(Path(drive.value))
        assert (mount / relative).read_bytes() == b"outside volume bytes"
        print(relative.as_posix())
except BaseException:
    if created:
        clear()
    raise
finally:
    if not close(handle):
        raise ctypes.WinError(ctypes.get_last_error())
