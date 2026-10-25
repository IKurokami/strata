#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Exercise the private GTK HGLOBAL stream without a display or clipboard.

The pinned private build retains COFF symbols. Resolve its internal constructor
from that exact DLL, rather than substituting a mock stream for the failing path.
"""

import argparse
import ctypes as c
import os
from pathlib import Path
import struct
import subprocess
import time

CF_HDROP = 15


class FormatPair(c.Structure):
    _fields_ = [("format", c.c_uint), ("mime", c.c_char_p), ("transmute", c.c_int)]


def bind(library, name, result, *arguments):
    function = getattr(library, name)
    function.restype = result
    function.argtypes = list(arguments)
    return function


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("runtime", type=Path)
    parser.add_argument("--expect-unsupported", action="store_true")
    args = parser.parse_args()
    path = args.runtime.resolve() / "libgtk-4-1.dll"
    directory = os.add_dll_directory(str(path.parent))
    gtk = c.CDLL(str(path))
    gio = c.CDLL("libgio-2.0-0.dll")
    glib = c.CDLL("libglib-2.0-0.dll")
    gobject = c.CDLL("libgobject-2.0-0.dll")
    kernel = c.WinDLL("kernel32", use_last_error=True)
    shell = c.WinDLL("shell32")
    get_path = bind(kernel, "GetModuleFileNameW", c.c_uint, c.c_void_p, c.c_wchar_p, c.c_uint)
    loaded = c.create_unicode_buffer(32768)
    assert get_path(gtk._handle, loaded, len(loaded))
    assert Path(loaded.value).resolve() == path
    print(f"GTK runtime: {loaded.value}", flush=True)

    image = path.read_bytes()
    pe = struct.unpack_from("<I", image, 0x3C)[0]
    assert image[pe:pe + 4] == b"PE\0\0"
    assert struct.unpack_from("<H", image, pe + 24)[0] == 0x20B, "x64 runtime required"
    image_base = struct.unpack_from("<Q", image, pe + 48)[0]
    symbols = {}
    for line in subprocess.check_output(["nm", "--defined-only", str(path)], text=True).splitlines():
        fields = line.split()
        if len(fields) == 3:
            symbols[fields[2]] = int(fields[0], 16) - image_base

    def internal(name, result, *arguments):
        offset = symbols[name]
        assert 0 < offset < len(image)
        return c.CFUNCTYPE(result, *arguments)(gtk._handle + offset)

    uses_hglobal = internal("_gdk_win32_format_uses_hdata", c.c_int, c.c_uint)
    if args.expect_unsupported:
        assert not uses_hglobal(CF_HDROP), "baseline unexpectedly supports CF_HDROP"
        print("Baseline CF_HDROP is incorrectly classified as a single handle", flush=True)
        return
    new_stream = internal("gdk_win32_hdata_output_stream_new", c.c_void_p,
                          c.c_void_p, c.POINTER(FormatPair), c.c_void_p)
    get_handle = internal("gdk_win32_hdata_output_stream_get_handle", c.c_void_p,
                          c.c_void_p, c.POINTER(c.c_int))
    write = bind(gio, "g_output_stream_write_all", c.c_int,
                 c.c_void_p, c.c_void_p, c.c_size_t, c.POINTER(c.c_size_t), c.c_void_p, c.c_void_p)
    close = bind(gio, "g_output_stream_close", c.c_int, c.c_void_p, c.c_void_p, c.c_void_p)
    unref = bind(gobject, "g_object_unref", None, c.c_void_p)
    free = bind(kernel, "GlobalFree", c.c_void_p, c.c_void_p)
    query = bind(shell, "DragQueryFileW", c.c_uint, c.c_void_p, c.c_uint, c.c_wchar_p, c.c_uint)
    callback_type = c.CFUNCTYPE(None, c.c_void_p, c.c_void_p, c.c_void_p)
    write_async = bind(gio, "g_output_stream_write_async", None, c.c_void_p,
                       c.c_void_p, c.c_size_t, c.c_int, c.c_void_p, callback_type, c.c_void_p)
    finish = bind(gio, "g_output_stream_write_finish", c.c_ssize_t,
                  c.c_void_p, c.c_void_p, c.c_void_p)
    iterate = bind(glib, "g_main_context_iteration", c.c_int, c.c_void_p, c.c_int)
    paths = [r"C:\Strata fixture\ảnh # 100%.png", r"\\server\shared folder\日本語.txt"]
    header = struct.pack("<IIIII", 20, 0, 0, 0, 1)
    chunks = [header] + [(name + "\0").encode("utf-16-le") for name in paths] + [b"\0\0"]
    for asynchronous in (False, True):
        pair = FormatPair(CF_HDROP, b"application/x.windows.CF_HDROP", 0)
        stream = new_stream(None, c.byref(pair), None)
        assert stream
        for chunk in chunks:
            buffer = c.create_string_buffer(chunk)
            if asynchronous:
                completed = []

                @callback_type
                def ready(source, result, _data):
                    completed.append(finish(source, result, None))

                write_async(stream, buffer, len(chunk), 0, None, ready, None)
                deadline = time.monotonic() + 15
                while not completed:
                    assert time.monotonic() < deadline, "stream callback timed out"
                    iterate(None, False)
                    time.sleep(0.001)
                assert completed == [len(chunk)]
            else:
                written = c.c_size_t()
                assert write(stream, buffer, len(chunk), c.byref(written), None, None)
                assert written.value == len(chunk)
        assert close(stream, None, None)
        is_hglobal = c.c_int()
        handle = get_handle(stream, c.byref(is_hglobal))
        assert handle and is_hglobal.value
        assert query(handle, 0xFFFFFFFF, None, 0) == len(paths)
        for index, expected in enumerate(paths):
            name = c.create_unicode_buffer(query(handle, index, None, 0) + 1)
            assert query(handle, index, name, len(name))
            assert name.value == expected
        assert free(handle) is None
        unref(stream)
        print(f"CF_HDROP {'async' if asynchronous else 'sync'} stream: native paths verified", flush=True)
    directory.close()


if __name__ == "__main__":
    main()
