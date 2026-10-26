//! Bounded parallel permanent deletion. Call only from a blocking worker.

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

fn remove_entry(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return if metadata.file_attributes() & 0x10 != 0 {
                fs::remove_dir(path)
            } else {
                fs::remove_file(path)
            };
        }
    }
    if metadata.is_dir() {
        // The standard library's recursive remover protects its traversal against
        // symlink replacement; do not replace it with a path-based recursive walk.
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

pub fn remove(path: &Path, cancelled: &AtomicBool) -> io::Result<()> {
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    if cancelled.load(Ordering::Acquire) {
        return Err(io::ErrorKind::Interrupted.into());
    }
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return remove_entry(path);
        }
    }
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return remove_entry(path);
    }

    #[cfg(windows)]
    let root_guard = {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        // Keep the selected root from being replaced by a junction while its
        // children are dispatched. Child removals still use std's protected walk.
        let guard = fs::OpenOptions::new()
            .read(true)
            .access_mode(0x80)
            .share_mode(1)
            .custom_flags(0x0200_0000 | 0x0020_0000)
            .open(path)?;
        if guard.metadata()?.file_attributes() & 0x400 != 0 {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        guard
    };

    let entries = fs::read_dir(path)?;
    let (sender, receiver) = mpsc::sync_channel::<PathBuf>(32);
    let receiver = Mutex::new(receiver);
    let failure = Mutex::new(None);
    let workers = thread::available_parallelism()
        .map_or(1, usize::from)
        .min(4);
    thread::scope(|scope| {
        let mut started = 0;
        for _ in 0..workers {
            let receiver = &receiver;
            let failure = &failure;
            let worker = thread::Builder::new()
                .name("strata-delete".into())
                .spawn_scoped(scope, move || {
                    loop {
                        let next = super::lock(receiver).recv();
                        let Ok(path) = next else { break };
                        if cancelled.load(Ordering::Acquire) {
                            continue;
                        }
                        if let Err(error) = remove_entry(&path) {
                            super::lock(failure).get_or_insert(error);
                        }
                    }
                });
            match worker {
                Ok(_) => started += 1,
                Err(error) => {
                    super::lock(failure).get_or_insert(error);
                    break;
                }
            }
        }
        if started == 0 {
            drop(sender);
            return;
        }
        for entry in entries {
            if cancelled.load(Ordering::Acquire) {
                break;
            }
            match entry {
                Ok(entry) => {
                    if sender.send(entry.path()).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    super::lock(&failure).get_or_insert(error);
                    break;
                }
            }
        }
        drop(sender);
    });
    if let Some(error) = super::lock(&failure).take() {
        return Err(error);
    }
    if cancelled.load(Ordering::Acquire) {
        return Err(io::ErrorKind::Interrupted.into());
    }
    #[cfg(windows)]
    drop(root_guard);
    fs::remove_dir(path)
}

#[cfg(test)]
mod tests;
