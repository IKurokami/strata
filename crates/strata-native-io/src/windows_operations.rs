//! Windows local file mutations with bounded, nonblocking submission.
//!
//! Unlike navigation and preview, file mutations are never dropped for
//! "latest wins": every accepted task completes in FIFO order. Explicit
//! operations run off the GTK thread without UAC, Shell hooks or ACL preflights.
//! Recycle Bin and permanent deletion are deliberately not implemented here.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread,
};

const MAX_QUEUED_OPERATIONS: usize = 32;
const MAX_QUEUED_COMPLETIONS: usize = 64;
const COPY_BUFFER_BYTES: usize = 128 * 1024;

/// A non-destructive local operation that Windows can execute without Shell
/// callbacks or an elevation request. Cross-volume move is intentionally not
/// hidden behind rename; the caller must decide how to handle that case.
#[derive(Debug)]
pub enum LocalOperation {
    CreateDirectory(PathBuf),
    CopyFile { source: PathBuf, destination: PathBuf },
    Rename { source: PathBuf, destination: PathBuf },
}

#[derive(Debug)]
pub struct OperationCompletion {
    pub id: u64,
    pub result: io::Result<()>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitError {
    Busy,
    Closed,
}

/// One worker preserves operation ordering. Neither submitting an operation
/// nor polling its completion performs filesystem I/O on the GTK thread.
pub struct LocalOperationService {
    sender: SyncSender<(u64, LocalOperation)>,
    updates: Receiver<OperationCompletion>,
    next_id: AtomicU64,
}

impl LocalOperationService {
    pub fn new() -> io::Result<Self> {
        let (sender, requests) = mpsc::sync_channel(MAX_QUEUED_OPERATIONS);
        let (completions, updates) = mpsc::sync_channel(MAX_QUEUED_COMPLETIONS);
        thread::Builder::new()
            .name("strata-windows-file-ops".into())
            .spawn(move || {
                while let Ok((id, operation)) = requests.recv() {
                    let result = execute(operation);
                    if completions.send(OperationCompletion { id, result }).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            sender,
            updates,
            next_id: AtomicU64::new(1),
        })
    }

    /// Fails fast when busy. Never retries or waits on GTK's UI thread.
    pub fn submit(&self, operation: LocalOperation) -> Result<u64, SubmitError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        match self.sender.try_send((id, operation)) {
            Ok(()) => Ok(id),
            Err(TrySendError::Full(_)) => Err(SubmitError::Busy),
            Err(TrySendError::Disconnected(_)) => Err(SubmitError::Closed),
        }
    }

    pub fn try_recv(&self) -> Result<OperationCompletion, TryRecvError> {
        self.updates.try_recv()
    }
}

fn execute(operation: LocalOperation) -> io::Result<()> {
    match operation {
        LocalOperation::CreateDirectory(path) => fs::create_dir(path),
        LocalOperation::CopyFile {
            source,
            destination,
        } => copy_no_clobber(source, destination),
        LocalOperation::Rename {
            source,
            destination,
        } => {
            // Native Windows rename does not silently fall back to a potentially
            // expensive cross-volume copy or escalate rights via Shell APIs.
            fs::rename(source, destination)
        }
    }
}

fn copy_no_clobber(source: PathBuf, destination: PathBuf) -> io::Result<()> {
    // Avoid accidentally reading a device or directory with streaming APIs.
    // All probes occur on the I/O worker, never on the UI thread.
    let source_metadata = fs::symlink_metadata(&source)?;
    if !source_metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "source must be an ordinary file, not a directory or symlink",
        ));
    }

    let mut reader = File::open(source)?;
    // CREATE_NEW on Windows: no existing file, ADS, or symlink target is overwritten.
    let mut writer = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)?;
    let result = (|| {
        let mut buffer = [0u8; COPY_BUFFER_BYTES];
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                return Ok(());
            }
            writer.write_all(&buffer[..read])?;
        }
    })();
    drop(writer);
    if result.is_err() {
        // Best-effort cleanup of a partial new file. The destination did not
        // exist before the exclusive create, so no original content is lost.
        let _ = fs::remove_file(destination);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::Path,
        sync::atomic::{AtomicUsize, Ordering},
        time::{Duration, Instant},
    };

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "strata-win-operations-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir(&dir).expect("make fixture");
            Self(dir)
        }
        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("clean test fixture");
        }
    }

    fn finished(service: &LocalOperationService, id: u64) -> io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match service.try_recv() {
                Ok(completion) if completion.id == id => return completion.result,
                Ok(_) | Err(TryRecvError::Empty) => {
                    assert!(Instant::now() < deadline, "operation worker timed out");
                    thread::sleep(Duration::from_millis(2));
                }
                Err(TryRecvError::Disconnected) => panic!("worker channel closed"),
            }
        }
    }

    #[test]
    fn creates_directories_off_the_ui_thread() {
        let dir = TempDir::new();
        let service = LocalOperationService::new().expect("start service");
        let path = dir.file("folder");
        let id = service
            .submit(LocalOperation::CreateDirectory(path.clone()))
            .expect("queue");
        finished(&service, id).expect("mkdir");
        assert!(Path::new(&path).is_dir());
    }

    #[test]
    fn copies_files_without_overwriting_destination() {
        let dir = TempDir::new();
        let src = dir.file("src.bin");
        let dest = dir.file("dest.bin");
        fs::write(&src, b"source bytes").expect("source");
        fs::write(&dest, b"keep me").expect("destination");
        let service = LocalOperationService::new().expect("service");
        let id = service
            .submit(LocalOperation::CopyFile {
                source: src,
                destination: dest.clone(),
            })
            .expect("queue");
        assert_eq!(
            finished(&service, id).expect_err("exclusive create must fail").kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(dest).expect("original destination"), b"keep me");
    }

    #[test]
    fn copies_large_files_into_new_destinations() {
        let dir = TempDir::new();
        let src = dir.file("src.bin");
        let dest = dir.file("dest.bin");
        let payload = vec![7u8; COPY_BUFFER_BYTES * 3 + 17];
        fs::write(&src, &payload).expect("source");
        let service = LocalOperationService::new().expect("service");
        let id = service
            .submit(LocalOperation::CopyFile {
                source: src,
                destination: dest.clone(),
            })
            .expect("queue");
        finished(&service, id).expect("copy");
        assert_eq!(fs::read(dest).expect("copy"), payload);
    }
}
