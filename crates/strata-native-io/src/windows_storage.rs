//! Native Windows atomic replacement of small application state files.
//!
//! Uses ordinary file handles and same-directory staging, never a shell,
//! UAC, Shell extensions, or Unix permissions. The final destination must be
//! a regular file without reparse attributes; we do not follow symlinks or
//! junctions in place of a file. Existing parent directory ACLs are inherited.
//!
//! Windows may refuse replacement if another program holds the destination
//! without delete sharing. Return that error and remove our staging file
//! rather than overwrite via a non-atomic fallback.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
const MAX_TEMPORARY_ATTEMPTS: usize = 64;
static NEXT_STAGING: AtomicU64 = AtomicU64::new(0);

fn validate_destination(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(info) => {
            if !info.is_file()
                || info.file_type().is_symlink()
                || info.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "refusing a non-regular or reparse-point destination",
                ));
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn create_temporary(parent: &Path) -> io::Result<(PathBuf, File)> {
    for _ in 0..MAX_TEMPORARY_ATTEMPTS {
        let id = NEXT_STAGING.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(".strata-write-{}-{id}.tmp", std::process::id()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "too many conflicting Strata staging files",
    ))
}

/// Replace a regular destination using a same-directory staging file.
///
/// This API performs disk I/O. Callers must not use it for slow/network paths
/// on GTK's UI thread. Unlike a read-only config lookup, durable file writes
/// include an explicit flush and sync.
pub fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    validate_destination(path)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "destination has no parent"))?;
    let (temporary, mut file) = create_temporary(parent)?;

    let written = file
        .write_all(contents)
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_all());
    // Windows cannot delete an open file without appropriate sharing flags.
    drop(file);
    if let Err(error) = written {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }

    let result = validate_destination(path).and_then(|()| fs::rename(&temporary, path));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Unlike Linux dotfile handling, Windows rejects symlink and junction targets
/// until an owner-verified reparse policy exists. Never silently follow one.
pub fn atomic_write_config(path: &Path, contents: &[u8]) -> io::Result<()> {
    atomic_write(path, contents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };
    static FIXTURE: AtomicUsize = AtomicUsize::new(0);

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "strata-win-storage-{}-{}-{}",
                std::process::id(),
                nonce,
                FIXTURE.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir(&path).expect("create scratch");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("remove scratch");
        }
    }

    #[test]
    fn atomically_creates_and_replaces_regular_file_without_leftovers() {
        let scratch = Scratch::new();
        let path = scratch.0.join("config.toml");
        atomic_write_config(&path, b"first").expect("create");
        assert_eq!(fs::read(&path).unwrap(), b"first");
        atomic_write_config(&path, b"second longer value").expect("replace");
        assert_eq!(fs::read(&path).unwrap(), b"second longer value");
        assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 1);
    }

    #[test]
    fn refuses_directory_destination_and_never_publishes_temp() {
        let scratch = Scratch::new();
        let directory = scratch.0.join("settings");
        fs::create_dir(&directory).unwrap();
        let error = atomic_write(&directory, b"not a file").expect_err("reject directory");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(directory.is_dir());
        assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 1);
    }

    #[test]
    fn missing_parent_does_not_create_directories() {
        let scratch = Scratch::new();
        let path = scratch.0.join("missing").join("settings.toml");
        assert!(atomic_write(&path, b"data").is_err());
        assert!(!path.parent().unwrap().exists());
        assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 0);
    }

    #[test]
    fn rejects_reparse_target_when_symlinks_are_permitted() {
        use std::os::windows::fs::symlink_file;
        let scratch = Scratch::new();
        let original = scratch.0.join("original.toml");
        let link = scratch.0.join("link.toml");
        fs::write(&original, b"original").unwrap();
        // Windows runners may lack symlink privilege, but in that case the
        // other non-regular destination tests still execute.
        if symlink_file(&original, &link).is_ok() {
            assert!(atomic_write_config(&link, b"modified").is_err());
            assert_eq!(fs::read(&original).unwrap(), b"original");
            assert!(
                fs::symlink_metadata(&link)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
        }
    }
}
