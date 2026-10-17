// SPDX-License-Identifier: MIT

//! Bounded native thumbnail cache for Windows.
//!
//! The Freedesktop cache cannot be reused on Win32 because its ownership and
//! no-follow guarantees are Unix-specific. This cache keeps entries under the
//! user's local application data directory, keys them by the volume/file
//! identity returned by Windows metadata, and stores a small envelope so a
//! stale or user-planted entry is only ever treated as a miss.

use sha2::{Digest, Sha256};
use std::{
    io::Read,
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
};

pub const CANONICAL_MAX_EDGE: i32 = 256;
const MAX_DISK_THUMBNAIL_BYTES: usize = 2 * 1024 * 1024;
const MAX_CACHE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CACHE_ENTRIES: usize = 512;
const MAX_CACHED_DIMENSION: u32 = 512;
const MAX_CACHED_PIXELS: u64 = 512 * 512;
const MAGIC: &[u8] = b"STRATA-THUMB-WIN\0";
const VERSION: u8 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileIdentity {
    volume: u32,
    index: u64,
    modified: u64,
    size: u64,
}

impl FileIdentity {
    fn read(path: &Path) -> Option<Self> {
        let metadata = std::fs::symlink_metadata(path).ok()?;
        if metadata.file_attributes() & 0x0000_0400 != 0 {
            return None;
        }
        let (volume, index) = strata_win_shell::file_identity(path).ok()?;
        Some(Self {
            volume,
            index,
            modified: metadata.last_write_time(),
            size: metadata.file_size(),
        })
    }

    fn encode(&self, output: &mut Vec<u8>) {
        output.extend_from_slice(&self.volume.to_le_bytes());
        output.extend_from_slice(&self.index.to_le_bytes());
        output.extend_from_slice(&self.modified.to_le_bytes());
        output.extend_from_slice(&self.size.to_le_bytes());
    }

    fn decode(bytes: &[u8]) -> Option<(Self, usize)> {
        let end = MAGIC.len() + 1 + 4 + 8 + 8 + 8;
        if bytes.len() < end || &bytes[..MAGIC.len()] != MAGIC || bytes[MAGIC.len()] != VERSION {
            return None;
        }
        let mut offset = MAGIC.len() + 1;
        let volume = u32::from_le_bytes(bytes[offset..offset + 4].try_into().ok()?);
        offset += 4;
        let index = u64::from_le_bytes(bytes[offset..offset + 8].try_into().ok()?);
        offset += 8;
        let modified = u64::from_le_bytes(bytes[offset..offset + 8].try_into().ok()?);
        offset += 8;
        let size = u64::from_le_bytes(bytes[offset..offset + 8].try_into().ok()?);
        offset += 8;
        Some((
            Self {
                volume,
                index,
                modified,
                size,
            },
            offset,
        ))
    }
}

pub fn lookup(path: &Path, _mtime: i64) -> Option<Vec<u8>> {
    let identity = FileIdentity::read(path)?;
    let cache = cache_path(&identity)?;
    let mut file = std::fs::File::open(cache).ok()?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take((MAX_DISK_THUMBNAIL_BYTES + MAGIC.len() + 64) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    let (stored, offset) = FileIdentity::decode(&bytes)?;
    if stored != identity {
        return None;
    }
    let png = bytes.get(offset..)?.to_vec();
    let (width, height) = png_dimensions(&png)?;
    if width == 0
        || height == 0
        || width > MAX_CACHED_DIMENSION
        || height > MAX_CACHED_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_CACHED_PIXELS
    {
        return None;
    }
    Some(png)
}

pub fn store(path: &Path, _mtime: i64, png: &[u8]) {
    if png.is_empty() || png.len() > MAX_DISK_THUMBNAIL_BYTES {
        return;
    }
    let Some(identity) = FileIdentity::read(path) else {
        return;
    };
    let Some((width, height)) = png_dimensions(png) else {
        return;
    };
    if width == 0
        || height == 0
        || width > MAX_CACHED_DIMENSION
        || height > MAX_CACHED_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_CACHED_PIXELS
    {
        return;
    }
    let Some(directory) = cache_directory() else {
        return;
    };
    if std::fs::create_dir_all(&directory).is_err() {
        return;
    }
    let destination = cache_path_in(&directory, &identity);
    let mut envelope = Vec::with_capacity(MAGIC.len() + 1 + 28 + png.len());
    envelope.extend_from_slice(MAGIC);
    envelope.push(VERSION);
    identity.encode(&mut envelope);
    envelope.extend_from_slice(png);

    let temporary = destination.with_extension(format!("{}.tmp", std::process::id()));
    if std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .and_then(|mut file| {
            use std::io::Write;
            file.write_all(&envelope)?;
            file.sync_all()
        })
        .is_ok()
    {
        let _ = std::fs::rename(&temporary, &destination);
    } else {
        let _ = std::fs::remove_file(&temporary);
    }
    trim_cache(&directory);
}

fn cache_directory() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|path| path.join("Strata").join("thumbnails").join("large"))
}

fn cache_path(identity: &FileIdentity) -> Option<PathBuf> {
    cache_directory().map(|directory| cache_path_in(&directory, identity))
}

fn cache_path_in(directory: &Path, identity: &FileIdentity) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(identity.volume.to_le_bytes());
    hasher.update(identity.index.to_le_bytes());
    hasher.update(identity.modified.to_le_bytes());
    hasher.update(identity.size.to_le_bytes());
    directory.join(format!("{}.thumb", hex::encode(hasher.finalize())))
}

fn trim_cache(directory: &Path) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut files: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "thumb"))
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            Some((entry.path(), metadata.len(), metadata.modified().ok()))
        })
        .collect();
    let mut total = files.iter().map(|(_, bytes, _)| *bytes).sum::<u64>();
    files.sort_by_key(|(_, _, modified)| *modified);
    while files.len() > MAX_CACHE_ENTRIES || total > MAX_CACHE_BYTES {
        let Some((path, bytes, _)) = files.first().cloned() else {
            break;
        };
        files.remove(0);
        total = total.saturating_sub(bytes);
        let _ = std::fs::remove_file(path);
    }
}

fn png_dimensions(png: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if png.get(..SIGNATURE.len())? != SIGNATURE || png.get(12..16)? != b"IHDR" {
        return None;
    }
    let length = u32::from_be_bytes(png.get(8..12)?.try_into().ok()?);
    if length < 13 || png.len() < 24 {
        return None;
    }
    let width = u32::from_be_bytes(png.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(png.get(20..24)?.try_into().ok()?);
    Some((width, height))
}
