//! Low-overhead, portable filesystem primitives for the native Windows port.
//!
//! No I/O or permission probes occur in `request`. The UI must use `try_recv`
//! and discard events whose generation is no longer the active selection.
//! This library deliberately does not decode or render untrusted preview data.
#![forbid(unsafe_code)]

use std::{
    ffi::OsString,
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread,
    time::Duration,
};

/// Names per event; file type and metadata are fetched lazily by the caller.
#[cfg(windows)]
pub mod windows_paths;

pub const LISTING_BATCH_SIZE: usize = 256;
/// Prevent unread UI events from consuming unbounded memory.
const LISTING_CHANNEL_CAPACITY: usize = 64;
const PREVIEW_CHANNEL_CAPACITY: usize = 8;
/// A raw prefetch bound, not a limit on the final parser-backed preview.
pub const MAX_RAW_PREVIEW_BYTES: usize = 1024 * 1024;

struct MailboxState<T> {
    pending: Option<(u64, T)>,
    stopping: bool,
}

/// One pending request per service; new requests replace stale pending work.
/// An OS call already in flight may finish, but its result cannot be published.
struct Mailbox<T> {
    latest: AtomicU64,
    state: Mutex<MailboxState<T>>,
    ready: Condvar,
}

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<T> Mailbox<T> {
    fn new() -> Self {
        Self {
            latest: AtomicU64::new(0),
            state: Mutex::new(MailboxState {
                pending: None,
                stopping: false,
            }),
            ready: Condvar::new(),
        }
    }

    fn submit(&self, task: T) -> u64 {
        let generation = self.latest.fetch_add(1, Ordering::AcqRel) + 1;
        let mut state = lock(&self.state);
        // The public API is thread-safe: an older submitter cannot overwrite
        // a newer pending request if their mutex acquisition is reordered.
        if state
            .pending
            .as_ref()
            .is_none_or(|(queued, _)| *queued < generation)
        {
            state.pending = Some((generation, task));
        }
        self.ready.notify_one();
        generation
    }

    fn next(&self) -> Option<(u64, T)> {
        let mut state = lock(&self.state);
        loop {
            if state.stopping {
                return None;
            }
            if let Some(task) = state.pending.take() {
                return Some(task);
            }
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn is_current(&self, generation: u64) -> bool {
        self.latest.load(Ordering::Acquire) == generation
    }

    fn shutdown(&self) {
        let mut state = lock(&self.state);
        state.stopping = true;
        state.pending = None;
        self.ready.notify_all();
    }
}

fn send_current<T, U>(
    sender: &SyncSender<U>,
    mailbox: &Mailbox<T>,
    generation: u64,
    mut event: U,
) -> bool {
    loop {
        if !mailbox.is_current(generation) {
            return false;
        }
        match sender.try_send(event) {
            Ok(()) => return true,
            Err(TrySendError::Disconnected(_)) => return false,
            Err(TrySendError::Full(pending)) => {
                event = pending;
                // Backpressure is applied to the worker, never to GTK.
                thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListingEntry {
    /// Preserve native platform names (including non-Unicode names on Unix).
    pub file_name: OsString,
}

#[derive(Debug)]
pub enum ListingEvent {
    Batch {
        generation: u64,
        entries: Vec<ListingEntry>,
    },
    Finished {
        generation: u64,
        error: Option<String>,
    },
}

/// Two workers allow a fresh local navigation even if one worker is blocked
/// inside an unresponsive network filesystem call. No worker can block GTK.
pub struct ListingService {
    mailbox: Arc<Mailbox<PathBuf>>,
    updates: Receiver<ListingEvent>,
}

impl ListingService {
    pub fn new() -> Self {
        let mailbox: Arc<Mailbox<PathBuf>> = Arc::new(Mailbox::new());
        let (sender, updates) = mpsc::sync_channel(LISTING_CHANNEL_CAPACITY);
        for index in 0..2 {
            let mailbox = Arc::clone(&mailbox);
            let sender = sender.clone();
            thread::Builder::new()
                .name(format!("strata-listing-{index}"))
                .spawn(move || {
                    while let Some((generation, path)) = mailbox.next() {
                        run_listing(&path, generation, &mailbox, &sender);
                    }
                })
                .expect("failed to start the filesystem listing worker");
        }
        Self { mailbox, updates }
    }

    /// Return immediately without probing existence, permissions or metadata.
    pub fn request(&self, path: impl Into<PathBuf>) -> u64 {
        self.mailbox.submit(path.into())
    }

    pub fn try_recv(&self) -> Result<ListingEvent, TryRecvError> {
        self.updates.try_recv()
    }
}

impl Default for ListingService {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ListingService {
    fn drop(&mut self) {
        // Never join workers on the UI thread: network I/O cannot always be
        // cancelled while the kernel is executing the underlying call.
        self.mailbox.shutdown();
    }
}

fn run_listing(
    path: &Path,
    generation: u64,
    mailbox: &Mailbox<PathBuf>,
    sender: &SyncSender<ListingEvent>,
) {
    let listing = match fs::read_dir(path) {
        Ok(listing) => listing,
        Err(error) => {
            let _ = send_current(
                sender,
                mailbox,
                generation,
                ListingEvent::Finished {
                    generation,
                    error: Some(error.to_string()),
                },
            );
            return;
        }
    };
    let mut batch = Vec::with_capacity(LISTING_BATCH_SIZE);
    for entry in listing {
        if !mailbox.is_current(generation) {
            return;
        }
        match entry {
            Ok(entry) => {
                // Avoid per-item stat, thumbnail, icon lookup or ACL probes.
                batch.push(ListingEntry {
                    file_name: entry.file_name(),
                });
                if batch.len() == LISTING_BATCH_SIZE
                    && !send_current(
                        sender,
                        mailbox,
                        generation,
                        ListingEvent::Batch {
                            generation,
                            entries: std::mem::replace(
                                &mut batch,
                                Vec::with_capacity(LISTING_BATCH_SIZE),
                            ),
                        },
                    )
                {
                    return;
                }
            }
            Err(error) => {
                let _ = send_current(
                    sender,
                    mailbox,
                    generation,
                    ListingEvent::Finished {
                        generation,
                        error: Some(error.to_string()),
                    },
                );
                return;
            }
        }
    }
    if !batch.is_empty()
        && !send_current(
            sender,
            mailbox,
            generation,
            ListingEvent::Batch {
                generation,
                entries: batch,
            },
        )
    {
        return;
    }
    let _ = send_current(
        sender,
        mailbox,
        generation,
        ListingEvent::Finished {
            generation,
            error: None,
        },
    );
}

struct PreviewTask {
    path: PathBuf,
    max_bytes: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RawPreview {
    pub bytes: Vec<u8>,
    pub truncated: bool,
}

#[derive(Debug)]
pub struct PreviewEvent {
    pub generation: u64,
    pub result: Result<RawPreview, String>,
}

/// A warmed worker accepts only the most recent selection. No raw-byte
/// preview parser runs here: untrusted formats require a separate sandbox.
pub struct PreviewService {
    mailbox: Arc<Mailbox<PreviewTask>>,
    updates: Receiver<PreviewEvent>,
}

impl PreviewService {
    pub fn new() -> Self {
        let mailbox: Arc<Mailbox<PreviewTask>> = Arc::new(Mailbox::new());
        let (sender, updates) = mpsc::sync_channel(PREVIEW_CHANNEL_CAPACITY);
        // A second worker can service a new selection while a stale file
        // open/read is stuck in an uncancellable OS or network call.
        for index in 0..2 {
            let worker = Arc::clone(&mailbox);
            let sender = sender.clone();
            thread::Builder::new()
                .name(format!("strata-preview-{index}"))
                .spawn(move || {
                    while let Some((generation, task)) = worker.next() {
                        let result = load_preview(&task, &worker, generation);
                        if let Some(result) = result {
                            let _ = send_current(
                                &sender,
                                &worker,
                                generation,
                                PreviewEvent {
                                    generation,
                                    result: result.map_err(|error| error.to_string()),
                                },
                            );
                        }
                    }
                })
                .expect("failed to start the preview I/O worker");
        }
        Self { mailbox, updates }
    }

    /// Latest request wins. Zero-length requests are permitted.
    pub fn request(&self, path: impl Into<PathBuf>, max_bytes: usize) -> u64 {
        self.mailbox.submit(PreviewTask {
            path: path.into(),
            max_bytes: max_bytes.min(MAX_RAW_PREVIEW_BYTES),
        })
    }

    pub fn try_recv(&self) -> Result<PreviewEvent, TryRecvError> {
        self.updates.try_recv()
    }
}

impl Default for PreviewService {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for PreviewService {
    fn drop(&mut self) {
        self.mailbox.shutdown();
    }
}

fn load_preview(
    task: &PreviewTask,
    mailbox: &Mailbox<PreviewTask>,
    generation: u64,
) -> Option<io::Result<RawPreview>> {
    let result = (|| {
        let mut file = File::open(&task.path)?;
        let mut bytes = Vec::with_capacity(task.max_bytes.min(8192));
        let mut buffer = [0u8; 8192];
        while bytes.len() < task.max_bytes {
            if !mailbox.is_current(generation) {
                // A superseded request must not do further file I/O.
                return Err(io::Error::new(io::ErrorKind::Interrupted, "superseded"));
            }
            let count = (task.max_bytes - bytes.len()).min(buffer.len());
            let read = file.read(&mut buffer[..count])?;
            if read == 0 {
                return Ok(RawPreview {
                    bytes,
                    truncated: false,
                });
            }
            bytes.extend_from_slice(&buffer[..read]);
        }
        // One extra byte distinguishes exact-size files from truncated reads.
        let mut extra = [0u8; 1];
        let truncated = file.read(&mut extra)? != 0;
        Ok(RawPreview { bytes, truncated })
    })();
    mailbox.is_current(generation).then_some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::{Duration, Instant},
    };

    static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "strata-native-io-{}-{}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).expect("create fixture directory");
            Self(path)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("remove fixture");
        }
    }

    fn wait_for<T>(mut f: impl FnMut() -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(result) = f() {
                return result;
            }
            assert!(Instant::now() < deadline, "timed out waiting for worker");
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn enumerates_names_in_bounded_batches_without_metadata() {
        let fixture = Fixture::new();
        for index in 0..600 {
            fs::write(fixture.0.join(format!("item-{index:04}")), b"x").expect("create test file");
        }
        let service = ListingService::new();
        let generation = service.request(fixture.0.clone());
        let mut all_names = Vec::new();
        wait_for(|| match service.try_recv() {
            Ok(ListingEvent::Batch {
                generation: id,
                entries,
            }) if id == generation => {
                assert!(entries.len() <= LISTING_BATCH_SIZE);
                all_names.extend(entries.into_iter().map(|entry| entry.file_name));
                None
            }
            Ok(ListingEvent::Finished {
                generation: id,
                error,
            }) if id == generation => {
                assert!(error.is_none(), "listing failed: {error:?}");
                Some(())
            }
            _ => None,
        });
        assert_eq!(all_names.len(), 600);
    }

    #[test]
    fn newest_listing_request_supersedes_prior_scan() {
        let fixture = Fixture::new();
        let empty = fixture.0.join("empty");
        fs::create_dir(&empty).expect("create empty directory");
        let service = ListingService::new();
        let previous = service.request(fixture.0.clone());
        let newest = service.request(&empty);
        assert!(newest > previous);
        wait_for(|| match service.try_recv() {
            Ok(ListingEvent::Finished {
                generation,
                error: None,
            }) if generation == newest => Some(()),
            _ => None,
        });
    }

    #[test]
    fn preview_is_bounded_and_reports_truncation() {
        let fixture = Fixture::new();
        let path = fixture.0.join("sample.bin");
        fs::write(&path, b"abcdefghijklmnopqrstuvwxyz").expect("create preview fixture");
        let service = PreviewService::new();
        let generation = service.request(path, 5);
        let result = wait_for(|| match service.try_recv() {
            Ok(event) if event.generation == generation => Some(event.result),
            _ => None,
        })
        .expect("preview read");
        assert_eq!(result.bytes, b"abcde");
        assert!(result.truncated);
    }

    #[test]
    fn preview_replaces_superseded_selection() {
        let fixture = Fixture::new();
        let first = fixture.0.join("first");
        let second = fixture.0.join("second");
        fs::write(&first, b"first").expect("first file");
        fs::write(&second, b"second").expect("second file");
        let service = PreviewService::new();
        let previous = service.request(&first, 5);
        let newest = service.request(&second, 6);
        assert!(newest > previous);
        let result = wait_for(|| match service.try_recv() {
            Ok(event) if event.generation == newest => Some(event.result),
            _ => None,
        })
        .expect("latest preview");
        assert_eq!(result.bytes, b"second");
        assert!(!result.truncated);
    }

    #[test]
    fn error_is_reported_without_a_permission_prompt() {
        let fixture = Fixture::new();
        let service = ListingService::new();
        let generation = service.request(fixture.0.join("missing"));
        let error = wait_for(|| match service.try_recv() {
            Ok(ListingEvent::Finished {
                generation: id,
                error,
            }) if id == generation => Some(error),
            _ => None,
        });
        assert!(error.is_some());
    }
}
