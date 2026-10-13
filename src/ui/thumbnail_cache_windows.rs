// SPDX-License-Identifier: MIT

//! Windows compatibility boundary for the Linux Freedesktop thumbnail cache.
//! The XDG thumbnail bucket requires Unix permission and O_NOFOLLOW behavior.
//! Do not read or write it on Windows, and do not decode untrusted thumbnail
//! bytes in-process. A Windows-native, handle-validated cache belongs behind
//! the future isolated preview renderer and must remain lazy.
use std::path::Path;

pub const CANONICAL_MAX_EDGE: i32 = 256;

/// No cross-process Freedesktop thumbnail cache exists on Windows.
pub fn lookup(_path: &Path, _mtime: i64) -> Option<Vec<u8>> {
    None
}

/// Fail closed rather than writing to an environment-selected Unix cache.
pub fn store(_path: &Path, _mtime: i64, _png: &[u8]) {}
