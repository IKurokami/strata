// SPDX-License-Identifier: MIT

use super::*;
use crate::rar_extraction::{self as wire, Failure, FailureKind, Record};
use std::{
    os::windows::process::CommandExt,
    process::{Command, Stdio},
    sync::atomic::AtomicU64,
    thread,
    time::Instant,
};

#[cfg(test)]
#[path = "rar/tests.rs"]
mod tests;

pub(super) fn extract_rar(
    archive: &Path,
    destination: &Path,
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
    cancelled: &Arc<AtomicBool>,
) -> Result<Option<String>, ArchiveError> {
    check_cancelled(cancelled)?;
    let secret = serde_json::to_vec(&password).map_err(|e| ArchiveError::Failed(e.to_string()))?;
    if password.is_some_and(|value| value.len() > crate::adapters::MAX_ARCHIVE_PASSWORD_BYTES) {
        return Err(ArchiveError::IncorrectPassword(
            "Archive password is too long.".to_owned(),
        ));
    }
    let executable = crate::services::installed_executable()
        .map_err(|e| ArchiveError::Failed(crate::services::io_error_message(&e)))?;
    // Only the decoder runs in the child. The parent validates paths and writes files.
    // This isolates decoder crashes; it does not claim AppContainer sandboxing.
    let mut child = Command::new(executable)
        .creation_flags(0x0800_0000)
        .args(["--preview-helper", "extract-rar"])
        .arg(archive)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| ArchiveError::Failed(crate::services::io_error_message(&e)))?;
    let input = child.stdin.take().expect("piped RAR secret input");
    let output = child.stdout.take().expect("piped RAR output");
    let done = Arc::new(AtomicBool::new(false));
    let activity = Arc::new(AtomicU64::new(0));
    let started = Instant::now();
    let watcher_done = done.clone();
    let watcher_activity = activity.clone();
    let watcher_cancelled = cancelled.clone();
    let watcher = thread::spawn(move || {
        loop {
            if watcher_done.load(Ordering::Relaxed)
                || watcher_cancelled.load(Ordering::Relaxed)
                || started.elapsed().as_millis() as u64
                    > watcher_activity
                        .load(Ordering::Relaxed)
                        .saturating_add(30_000)
            {
                let _ = child.kill();
                return child.wait();
            }
            match child.try_wait() {
                Ok(Some(status)) => return Ok(status),
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error);
                }
                Ok(None) => thread::sleep(Duration::from_millis(20)),
            }
        }
    });
    let result = (|| {
        let mut input = input;
        input
            .write_all(&secret)
            .map_err(|e| ArchiveError::Failed(e.to_string()))?;
        drop(input);
        let reader = ActiveReader {
            inner: output,
            activity,
            started,
        };
        extract_stream(reader, destination, progress, cancelled)
    })();
    done.store(true, Ordering::Relaxed);
    let status = watcher.join().map_err(|_| {
        ArchiveError::Failed(crate::i18n::tr(
            "This file is not a valid archive or is damaged.",
        ))
    })?;
    check_cancelled(cancelled)?;
    // End-of-stream records are emitted after successful decoding; stopping a child
    // that has already emitted one is safe even if its final process exit races us.
    if result.is_ok() && status.is_err() {
        return Err(ArchiveError::Failed(crate::i18n::tr(
            "This file is not a valid archive or is damaged.",
        )));
    }
    result
}

struct ActiveReader<R> {
    inner: R,
    activity: Arc<AtomicU64>,
    started: Instant,
}

impl<R: Read> Read for ActiveReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.inner.read(buffer)?;
        if count > 0 {
            self.activity
                .store(self.started.elapsed().as_millis() as u64, Ordering::Relaxed);
        }
        Ok(count)
    }
}

fn wire_error(error: Failure) -> ArchiveError {
    match error.kind {
        FailureKind::PasswordRequired => ArchiveError::PasswordRequired(error.message),
        FailureKind::IncorrectPassword => ArchiveError::IncorrectPassword(error.message),
        FailureKind::Other => ArchiveError::Failed(crate::i18n::tr(&error.message)),
    }
}

fn extract_stream(
    mut reader: impl Read,
    destination: &Path,
    progress: &AtomicUsize,
    cancelled: &AtomicBool,
) -> Result<Option<String>, ArchiveError> {
    wire::read_magic(&mut reader).map_err(|e| ArchiveError::Failed(e.to_string()))?;
    let mut resolver = WindowsExtractResolver::new();
    let mut first_name = None;
    loop {
        check_cancelled(cancelled)?;
        let record =
            wire::read_record(&mut reader).map_err(|e| ArchiveError::Failed(e.to_string()))?;
        let (name, size) = match record {
            Record::End => return Ok(first_name),
            Record::Error(failure) => return Err(wire_error(failure)),
            Record::Directory(name, _) => (name, None),
            Record::File(name, size, _) => (name, Some(size)),
        };
        let relative = sanitize_archive_member_path(&name).map_err(ArchiveError::Failed)?;
        let relative = resolver
            .resolve(destination, &relative)
            .map_err(ArchiveError::Failed)?;
        let target = destination.join(&relative);
        if first_name.is_none() {
            first_name = relative
                .components()
                .next()
                .map(|c| c.as_os_str().to_string_lossy().into_owned());
        }
        if let Some(size) = size {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| ArchiveError::Failed(e.to_string()))?;
            }
            let mut file = File::options()
                .write(true)
                .create_new(true)
                .open(&target)
                .map_err(|e| ArchiveError::Failed(e.to_string()))?;
            let result: Result<(), ArchiveError> = (|| {
                let mut body = wire::FileBody::new(&mut reader, size);
                let mut buffer = vec![0; COPY_BUF_SIZE];
                loop {
                    check_cancelled(cancelled)?;
                    let count = body
                        .read(&mut buffer)
                        .map_err(|e| wire_error(Failure::from_io(e)))?;
                    if count == 0 {
                        break;
                    }
                    file.write_all(&buffer[..count])
                        .map_err(|e| ArchiveError::Failed(e.to_string()))?;
                }
                Ok(())
            })();
            drop(file);
            if result.is_err() {
                let _ = std::fs::remove_file(&target);
            }
            result?;
        } else {
            std::fs::create_dir_all(&target).map_err(|e| ArchiveError::Failed(e.to_string()))?;
        }
        progress.fetch_add(1, Ordering::Relaxed);
    }
}
