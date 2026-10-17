//! Directory merge and verified cross-volume move.
//!
//! Neither operation ever replaces an existing destination file, and a move
//! deletes the source only after the destination copy was verified.

use super::{
    AtomicU8, FILE_ATTRIBUTE_REPARSE_POINT, OP_CANCELLED, Ordering, copy_no_clobber,
    copy_path_no_clobber,
};
use std::{
    fs::{self, File},
    io::{self, Read},
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
};

const VERIFY_BUFFER_BYTES: usize = 64 * 1024;

fn cancelled_error() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "operation cancelled")
}

fn check_cancelled(cancelled: &AtomicU8) -> io::Result<()> {
    if cancelled.load(Ordering::Acquire) == OP_CANCELLED {
        Err(cancelled_error())
    } else {
        Ok(())
    }
}

fn unsupported_entry() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "source contains a reparse point or unsupported filesystem entry",
    )
}

/// Merge `source` into `destination`. Existing destination directories are
/// reused (and must not be reparse points); an existing destination file is a
/// collision and fails without being modified.
pub(super) fn merge_path(
    source: &Path,
    destination: &Path,
    cancelled: &AtomicU8,
) -> io::Result<()> {
    check_cancelled(cancelled)?;
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(unsupported_entry());
    }
    if metadata.is_file() {
        // Child copies are already past the merge's commit point.
        return copy_no_clobber(
            source.to_path_buf(),
            destination.to_path_buf(),
            &AtomicU8::new(0),
        );
    }
    if !metadata.is_dir() {
        return Err(unsupported_entry());
    }
    match fs::create_dir(destination) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let existing = fs::symlink_metadata(destination)?;
            if !existing.is_dir() || existing.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            {
                return Err(error);
            }
        }
        Err(error) => return Err(error),
    }
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        merge_path(
            &entry.path(),
            &destination.join(entry.file_name()),
            cancelled,
        )?;
    }
    Ok(())
}

/// Copy, verify byte-for-byte, then delete the source. A cancellation can only
/// win before the copy publishes; afterwards the move runs to completion.
pub(super) fn move_verified(
    source: &Path,
    destination: &Path,
    cancelled: &AtomicU8,
) -> io::Result<()> {
    copy_path_no_clobber(source.to_path_buf(), destination.to_path_buf(), cancelled)?;
    if let Err(error) = verify_same(source, destination) {
        // This operation created the destination (no-clobber), so it owns it.
        let _ = if destination.is_dir() {
            fs::remove_dir_all(destination)
        } else {
            fs::remove_file(destination)
        };
        return Err(error);
    }
    delete_verified_source(source)
}

fn verify_same(source: &Path, destination: &Path) -> io::Result<()> {
    let source_meta = fs::symlink_metadata(source)?;
    let destination_meta = fs::symlink_metadata(destination)?;
    if source_meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || destination_meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(unsupported_entry());
    }
    if source_meta.is_file() && destination_meta.is_file() {
        return verify_file_contents(source, destination, source_meta.len());
    }
    if !(source_meta.is_dir() && destination_meta.is_dir()) {
        return Err(mismatch());
    }
    let mut count = 0usize;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        verify_same(&entry.path(), &destination.join(entry.file_name()))?;
        count += 1;
    }
    if fs::read_dir(destination)?.count() != count {
        return Err(mismatch());
    }
    Ok(())
}

fn mismatch() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "destination copy does not match the source; source kept",
    )
}

fn verify_file_contents(source: &Path, destination: &Path, length: u64) -> io::Result<()> {
    let mut left = File::open(source)?;
    let mut right = File::open(destination)?;
    if right.metadata()?.len() != length {
        return Err(mismatch());
    }
    let mut a = vec![0u8; VERIFY_BUFFER_BYTES];
    let mut b = vec![0u8; VERIFY_BUFFER_BYTES];
    loop {
        let read = left.read(&mut a)?;
        if read == 0 {
            return if right.read(&mut b)? == 0 {
                Ok(())
            } else {
                Err(mismatch())
            };
        }
        right.read_exact(&mut b[..read]).map_err(|_| mismatch())?;
        if a[..read] != b[..read] {
            return Err(mismatch());
        }
    }
}

/// Bottom-up removal with non-recursive directory removal: an entry that
/// appeared in the source after verification makes `remove_dir` fail and is
/// preserved instead of being deleted unseen.
fn delete_verified_source(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(unsupported_entry());
    }
    if metadata.is_file() {
        return fs::remove_file(path);
    }
    for entry in fs::read_dir(path)? {
        delete_verified_source(&entry?.path())?;
    }
    fs::remove_dir(path)
}

const MAX_KEEP_BOTH_ATTEMPTS: u32 = 10_000;

use crate::windows_paths::numbered_candidate;

/// Copy without replacing anything, choosing the first free numbered name.
/// Returns the path that was actually published. `AlreadyExists` from the
/// atomic publish (a concurrent creator) simply advances to the next name.
pub(super) fn copy_path_keep_both(
    source: &Path,
    destination: &Path,
    cancelled: &AtomicU8,
) -> io::Result<PathBuf> {
    let is_file = fs::symlink_metadata(source)?.is_file();
    let mut candidate = destination.to_path_buf();
    for index in 1..=MAX_KEEP_BOTH_ATTEMPTS {
        if fs::symlink_metadata(&candidate).is_err() {
            match copy_path_no_clobber(source.to_path_buf(), candidate.clone(), cancelled) {
                Ok(()) => return Ok(candidate),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        check_cancelled(cancelled)?;
        candidate = numbered_candidate(destination, index, is_file);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free numbered name was available",
    ))
}
#[cfg(test)]
#[path = "merge_move/tests.rs"]
mod tests;
