//! Windows local file mutations with bounded, nonblocking submission.
//!
//! Unlike navigation and preview, file mutations are never dropped for
//! "latest wins": every accepted task completes in FIFO order. Explicit
//! operations run off the GTK thread without UAC, Shell hooks or ACL preflights.
//! Recycle Bin and permanent deletion are deliberately not implemented here.
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_QUEUED_OPERATIONS: usize = 32;
const MAX_QUEUED_REMOTE_OPERATIONS: usize = 8;
const MAX_QUEUED_COMPLETIONS: usize = 64;
const COPY_BUFFER_BYTES: usize = 128 * 1024;
// An accepted cancellation must win the same CAS as final publication.
const OP_PENDING: u8 = 0;
const OP_CANCELLED: u8 = 1;
const OP_COMMITTING: u8 = 2;
type QueuedOperation = (u64, LocalOperation, Arc<AtomicU8>);
type OutstandingOperations = Arc<Mutex<HashMap<u64, Arc<AtomicU8>>>>;

fn request_cancel(state: &AtomicU8) -> bool {
    state
        .compare_exchange(
            OP_PENDING,
            OP_CANCELLED,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_ok()
}

fn start_irreversible_step(state: &AtomicU8) -> io::Result<()> {
    state
        .compare_exchange(
            OP_PENDING,
            OP_COMMITTING,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .map(|_| ())
        .map_err(|_| io::Error::new(io::ErrorKind::Interrupted, "operation cancelled"))
}
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
static NEXT_STAGING_ID: AtomicU64 = AtomicU64::new(0);

/// Non-destructive local operations that Windows can execute without Shell
/// callbacks or elevation. Rename/move and Recycle Bin remain in the
/// full operation provider until no-clobber semantics are implemented.
#[derive(Debug)]
pub enum LocalOperation {
    CreateDirectory(PathBuf),
    CopyFile {
        source: PathBuf,
        destination: PathBuf,
    },
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

/// One worker per backend preserves submission ordering within that backend.
/// UNC shares are isolated: a hung network copy cannot block a local mkdir.
/// Neither submission nor polling performs filesystem I/O on GTK's thread.
pub struct LocalOperationService {
    sender: SyncSender<QueuedOperation>,
    remote_sender: SyncSender<QueuedOperation>,
    updates: Receiver<OperationCompletion>,
    outstanding: OutstandingOperations,
    next_id: AtomicU64,
}

impl LocalOperationService {
    pub fn new() -> io::Result<Self> {
        let (sender, requests) = mpsc::sync_channel(MAX_QUEUED_OPERATIONS);
        let (remote_sender, remote_requests) = mpsc::sync_channel(MAX_QUEUED_REMOTE_OPERATIONS);
        let (completions, updates) = mpsc::sync_channel(MAX_QUEUED_COMPLETIONS);
        let outstanding: OutstandingOperations = Arc::new(Mutex::new(HashMap::new()));
        spawn_operation_worker(
            "strata-windows-file-ops",
            requests,
            completions.clone(),
            Arc::clone(&outstanding),
        )?;
        spawn_operation_worker(
            "strata-windows-unc-ops",
            remote_requests,
            completions,
            Arc::clone(&outstanding),
        )?;
        Ok(Self {
            sender,
            remote_sender,
            updates,
            outstanding,
            next_id: AtomicU64::new(1),
        })
    }

    /// Fails fast when busy. Never retries or waits on GTK's UI thread.
    pub fn submit(&self, operation: LocalOperation) -> Result<u64, SubmitError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let queue = if operation_uses_unc(&operation) {
            &self.remote_sender
        } else {
            &self.sender
        };
        let cancelled = Arc::new(AtomicU8::new(OP_PENDING));
        self.outstanding
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, Arc::clone(&cancelled));
        match queue.try_send((id, operation, cancelled)) {
            Ok(()) => Ok(id),
            Err(error) => {
                self.outstanding
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&id);
                match error {
                    TrySendError::Full(_) => Err(SubmitError::Busy),
                    TrySendError::Disconnected(_) => Err(SubmitError::Closed),
                }
            }
        }
    }

    /// Cooperatively cancel a queued or active operation. A kernel read already
    /// in progress cannot be interrupted, but no further copy chunk or final
    /// publication occurs once the worker observes this flag.
    pub fn cancel(&self, id: u64) -> bool {
        let entries = self
            .outstanding
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(cancelled) = entries.get(&id) {
            request_cancel(cancelled)
        } else {
            false
        }
    }

    pub fn try_recv(&self) -> Result<OperationCompletion, TryRecvError> {
        self.updates.try_recv()
    }
}

fn operation_uses_unc(operation: &LocalOperation) -> bool {
    use crate::windows_paths::{PathKind, classify};
    let remote = |path: &Path| classify(path) == PathKind::NetworkAbsolute;
    match operation {
        LocalOperation::CreateDirectory(path) => remote(path),
        LocalOperation::CopyFile {
            source,
            destination,
        } => remote(source) || remote(destination),
    }
}

fn spawn_operation_worker(
    name: &str,
    requests: Receiver<QueuedOperation>,
    completions: SyncSender<OperationCompletion>,
    outstanding: OutstandingOperations,
) -> io::Result<()> {
    thread::Builder::new().name(name.into()).spawn(move || {
        while let Ok((id, operation, cancelled)) = requests.recv() {
            let result = if cancelled.load(Ordering::Acquire) == OP_CANCELLED {
                Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "operation cancelled",
                ))
            } else {
                execute(operation, &cancelled)
            };
            outstanding
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&id);
            if completions
                .send(OperationCompletion { id, result })
                .is_err()
            {
                break;
            }
        }
    })?;
    Ok(())
}

/// Refuse working-directory-relative and Windows device namespace paths.
/// A UI-selected operation must refer to an explicit absolute filesystem path,
/// not a process-global current-directory or a named device.
fn validate_native_file_path(path: &Path) -> io::Result<()> {
    use crate::windows_paths::{PathKind, classify, valid_child_name};

    if !matches!(
        classify(path),
        PathKind::LocalAbsolute | PathKind::NetworkAbsolute
    ) || !path.file_name().is_some_and(valid_child_name)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected an absolute ordinary Windows filesystem path",
        ));
    }
    Ok(())
}

fn execute(operation: LocalOperation, cancelled: &AtomicU8) -> io::Result<()> {
    match operation {
        LocalOperation::CreateDirectory(path) => {
            validate_native_file_path(&path)?;
            start_irreversible_step(cancelled)?;
            fs::create_dir(path)
        }
        LocalOperation::CopyFile {
            source,
            destination,
        } => {
            validate_native_file_path(&source)?;
            validate_native_file_path(&destination)?;
            copy_no_clobber(source, destination, cancelled)
        }
    }
}

/// Open the source handle without following the final NTFS symlink/reparse
/// point. Checking the opened handle, not the path before open, closes the
/// final-component TOCTOU race; intermediate parent junctions still resolve.
pub(crate) fn open_regular_source(path: &Path) -> io::Result<File> {
    validate_native_file_path(path)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "source must be an ordinary file, not a directory or reparse point",
        ));
    }
    Ok(file)
}

/// Each request creates an exclusive staging file in the destination folder.
/// Publishing is an atomic no-overwrite NTFS hardlink, never an unsafe rename.
/// On volumes without compatible hardlinks, this operation fails without
/// touching the final destination (a slow fallback must be explicit).
fn copy_no_clobber(
    source: PathBuf,
    destination: PathBuf,
    cancelled: &AtomicU8,
) -> io::Result<()> {
    let mut reader = open_regular_source(&source)?;
    stage_and_publish(&mut reader, &destination, cancelled)
}

struct StagingFile {
    file: Option<File>,
    path: PathBuf,
}

impl Drop for StagingFile {
    fn drop(&mut self) {
        // The randomly named scratch file is never the user's requested
        // destination. This best-effort cleanup does not touch the final link.
        self.file.take();
        let _ = fs::remove_file(&self.path);
    }
}

fn new_staging_file(destination: &Path) -> io::Result<StagingFile> {
    let directory = destination
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "destination has no parent"))?;
    // Per-process monotonic ID plus time is a collision-resistant scratch
    // filename; CREATE_NEW remains the authority for concurrent processes.
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for _ in 0..16 {
        let counter = NEXT_STAGING_ID.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!(
            ".strata-stage-{}-{seed:x}-{counter:x}.tmp",
            std::process::id(),
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => {
                return Ok(StagingFile {
                    file: Some(file),
                    path,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not reserve a unique staging filename",
    ))
}

fn stage_and_publish(
    reader: &mut impl Read,
    destination: &Path,
    cancelled: &AtomicU8,
) -> io::Result<()> {
    let mut staging = new_staging_file(destination)?;
    let file = staging.file.as_mut().expect("staging writer exists");
    let mut buffer = [0u8; COPY_BUFFER_BYTES];
    loop {
        if cancelled.load(Ordering::Acquire) == OP_CANCELLED {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "copy cancelled"));
        }
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])?;
    }
    file.flush()?;
    // Publish and cancel both use one CAS: cancel(id) can no longer claim
    // success once the worker has entered irreversible NTFS publication.
    start_irreversible_step(cancelled)?;
    // Atomic publication, only once the entire source was read successfully.
    // Hard-link creation fails with AlreadyExists if the destination is
    // concurrently created, and never replaces an existing file.
    fs::hard_link(&staging.path, destination)
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
    fn direct_unc_mutations_route_only_to_the_network_worker() {
        assert!(operation_uses_unc(&LocalOperation::CreateDirectory(
            PathBuf::from(r"\\server\share\new-folder")
        )));
        assert!(!operation_uses_unc(&LocalOperation::CreateDirectory(
            PathBuf::from(r"C:\Users\Public\new-folder")
        )));
        assert!(operation_uses_unc(&LocalOperation::CopyFile {
            source: PathBuf::from(r"\\server\share\remote.txt"),
            destination: PathBuf::from(r"C:\Users\Public\local.txt"),
        }));
        assert!(operation_uses_unc(&LocalOperation::CopyFile {
            source: PathBuf::from(r"C:\Users\Public\local.txt"),
            destination: PathBuf::from(r"\\server\share\remote.txt"),
        }));
    }

    #[test]
    fn cancellation_cannot_claim_success_after_publication_begins() {
        let before_commit = AtomicU8::new(OP_PENDING);
        assert!(request_cancel(&before_commit));
        assert!(start_irreversible_step(&before_commit).is_err());

        let after_commit = AtomicU8::new(OP_PENDING);
        start_irreversible_step(&after_commit).expect("start commit");
        assert!(!request_cancel(&after_commit));
        assert_eq!(after_commit.load(Ordering::Acquire), OP_COMMITTING);
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
            finished(&service, id)
                .expect_err("exclusive create must fail")
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(dest).expect("original destination"), b"keep me");
    }

    #[test]
    fn rejects_relative_and_device_paths_without_touching_them() {
        let dir = TempDir::new();
        let service = LocalOperationService::new().expect("service");
        for destination in [
            PathBuf::from(r"some-relative-folder"),
            PathBuf::from(r"C:drive-relative"),
            PathBuf::from(r"\\.\pipe\strata-test"),
        ] {
            let id = service
                .submit(LocalOperation::CreateDirectory(destination))
                .expect("queue");
            assert_eq!(
                finished(&service, id)
                    .expect_err("reject invalid path")
                    .kind(),
                io::ErrorKind::InvalidInput
            );
        }
        let id = service
            .submit(LocalOperation::CopyFile {
                source: PathBuf::from("relative-src"),
                destination: dir.file("never-created"),
            })
            .expect("queue");
        assert_eq!(
            finished(&service, id)
                .expect_err("reject relative source")
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(!dir.file("never-created").exists());
    }

    #[test]
    fn copying_directory_is_rejected_without_creating_destination() {
        let dir = TempDir::new();
        let destination = dir.file("new-file");
        let service = LocalOperationService::new().expect("service");
        let id = service
            .submit(LocalOperation::CopyFile {
                source: dir.0.clone(),
                destination: destination.clone(),
            })
            .expect("queue");
        let error = finished(&service, id).expect_err("directory is not a file");
        assert!(
            matches!(
                error.kind(),
                io::ErrorKind::InvalidInput | io::ErrorKind::PermissionDenied
            ),
            "a directory must be rejected before staging: {error}"
        );
        assert!(!destination.exists());
    }

    #[test]
    fn failed_reader_does_not_publish_partial_destination() {
        struct FailAfterFirstRead(bool);
        impl Read for FailAfterFirstRead {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.0 {
                    return Err(io::Error::other("injected source failure"));
                }
                self.0 = true;
                buf[0] = 42;
                Ok(1)
            }
        }
        let dir = TempDir::new();
        let dest = dir.file("not-published");
        let err = stage_and_publish(
            &mut FailAfterFirstRead(false),
            &dest,
            &AtomicU8::new(OP_PENDING),
        )
        .expect_err("injected copy must fail");
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert!(
            !dest.exists(),
            "failed staging must not publish a partial file"
        );
        assert_eq!(
            fs::read_dir(&dir.0).expect("list scratch").count(),
            0,
            "failed staging must release the scratch file"
        );
    }

    #[test]
    fn final_destination_created_during_copy_is_never_replaced() {
        struct PublishExisting<'a> {
            destination: &'a Path,
            sent: bool,
        }
        impl Read for PublishExisting<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.sent {
                    return Ok(0);
                }
                self.sent = true;
                fs::write(self.destination, b"created concurrently")?;
                buf[0] = 7;
                Ok(1)
            }
        }
        let dir = TempDir::new();
        let dest = dir.file("concurrent.txt");
        let result = stage_and_publish(
            &mut PublishExisting {
                destination: &dest,
                sent: false,
            },
            &dest,
            &AtomicU8::new(OP_PENDING),
        );
        assert_eq!(
            result.expect_err("destination collision").kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(
            fs::read(&dest).expect("read concurrent file"),
            b"created concurrently"
        );
        assert_eq!(fs::read_dir(&dir.0).expect("list").count(), 1);
    }

    #[test]
    fn explicit_cancel_stops_before_any_final_filename_is_published() {
        struct CancelAfterFirstRead<'a>(&'a AtomicU8, bool);
        impl Read for CancelAfterFirstRead<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.1 {
                    return Ok(0);
                }
                self.1 = true;
                buf[0] = 99;
                self.0.store(OP_CANCELLED, Ordering::Release);
                Ok(1)
            }
        }
        let dir = TempDir::new();
        let destination = dir.file("not-published");
        let cancelled = AtomicU8::new(OP_PENDING);
        let error = stage_and_publish(
            &mut CancelAfterFirstRead(&cancelled, false),
            &destination,
            &cancelled,
        )
        .expect_err("copy cancelled before publish");
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(!destination.exists());
        assert_eq!(fs::read_dir(&dir.0).expect("list directory").count(), 0);
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
