// SPDX-License-Identifier: MIT

use std::{
    collections::HashSet,
    ffi::{OsStr, OsString},
    fs,
    path::Path,
};

use crate::model::EntryKind;

#[cfg(target_os = "linux")]
use std::{io::Read, os::unix::ffi::OsStringExt};

#[cfg(target_os = "linux")]
const MAX_HIDDEN_FILE_BYTES: u64 = 1024 * 1024;

pub(crate) fn native_kind(file_type: fs::FileType, path: &Path) -> EntryKind {
    if file_type.is_dir() {
        return EntryKind::Directory;
    }
    if file_type.is_file() {
        return EntryKind::File;
    }
    if !file_type.is_symlink() {
        return EntryKind::Other;
    }
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => EntryKind::DirectorySymbolicLink,
        Ok(metadata) if metadata.is_file() => EntryKind::FileSymbolicLink,
        Ok(_) => EntryKind::Other,
        Err(_) => EntryKind::SymbolicLink,
    }
}

pub(crate) fn is_hidden_name(native_name: &OsStr, hidden_names: &HashSet<OsString>) -> bool {
    native_name.as_encoded_bytes().first().copied() == Some(b'.')
        || hidden_names.contains(native_name)
}

// Windows hidden files use FILE_ATTRIBUTE_HIDDEN via GIO file metadata;
// scanning a GNOME .hidden file is not applicable and would waste I/O.
#[cfg(target_os = "windows")]
pub(crate) fn native_hidden_names(_path: &Path) -> HashSet<OsString> {
    HashSet::new()
}

#[cfg(target_os = "linux")]
pub(crate) fn native_hidden_names(path: &Path) -> HashSet<OsString> {
    let Some(bytes) = read_hidden_bounded(&path.join(".hidden")) else {
        return HashSet::new();
    };
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|name| !name.is_empty())
        .map(|name| OsString::from_vec(name.strip_suffix(b"\r").unwrap_or(name).to_vec()))
        .collect()
}

#[cfg(target_os = "linux")]
fn read_hidden_bounded(path: &Path) -> Option<Vec<u8>> {
    // Opening a FIFO without `NONBLOCK` would block waiting for a writer.
    let fd = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NONBLOCK,
        rustix::fs::Mode::empty(),
    )
    .ok()?;
    let stat = rustix::fs::fstat(&fd).ok()?;
    if rustix::fs::FileType::from_raw_mode(stat.st_mode) != rustix::fs::FileType::RegularFile {
        return None;
    }
    let file = std::fs::File::from(fd);
    let mut bytes = Vec::new();
    file.take(MAX_HIDDEN_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_HIDDEN_FILE_BYTES {
        return None;
    }
    Some(bytes)
}
