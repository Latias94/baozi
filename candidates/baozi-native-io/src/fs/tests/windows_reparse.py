"""Create actual unknown-tag fixtures inside one caller-created test directory."""

import ctypes
from ctypes import wintypes
from pathlib import Path
import struct
import sys
import uuid


def set_reparse(path: Path, tag: int) -> None:
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
    # Open the fixture itself, including directories, without processing its tag.
    handle = create(str(path), 0xC0000000, 7, None, 3, 0x02200000, None)
    if handle == wintypes.HANDLE(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    guid = uuid.UUID("3cc99342-c02f-4e78-97ec-6da99572c105").bytes_le
    data = ctypes.create_string_buffer(struct.pack("<IHH16s", tag, 0, 0, guid))
    returned = wintypes.DWORD()
    try:
        if not control(handle, 0x000900A4, data, 24, None, 0,
                       ctypes.byref(returned), None):
            raise ctypes.WinError(ctypes.get_last_error())
        observed = ctypes.create_string_buffer(16384)
        if not control(handle, 0x000900A8, None, 0, observed, len(observed),
                       ctypes.byref(returned), None):
            raise ctypes.WinError(ctypes.get_last_error())
        if observed.raw[:24] != data.raw[:24]:
            raise RuntimeError("reparse tag/GUID readback differs from the created fixture")
    finally:
        if not close(handle):
            raise ctypes.WinError(ctypes.get_last_error())


root = Path(sys.argv[1]).resolve(strict=True)
with (root / "ordinary.bin").open("xb") as output:
    output.write(b"authorized ordinary file")
with (root / "unknown.bin").open("xb") as output:
    output.write(b"opaque reparse payload")
set_reparse(root / "unknown.bin", 0x00000042)

directory = root / "tagged"
directory.mkdir()
# The directory bit permits ordinary children without assigning redirect semantics.
set_reparse(directory, 0x10000042)
with (directory / "ordinary.bin").open("xb") as output:
    output.write(b"authorized directory child")
with (directory / "unknown.bin").open("xb") as output:
    output.write(b"nested opaque reparse payload")
set_reparse(directory / "unknown.bin", 0x00000042)
