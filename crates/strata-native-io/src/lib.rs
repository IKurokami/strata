//! Low-overhead, portable filesystem primitives for the native Windows port.
//!
//! No I/O or permission probes occur in `request`. The UI must use `try_recv`
//! and discard events whose generation is no longer the active selection.
//! This library deliberately does not decode or render untrusted preview data.
#![forbid(unsafe_code)]

use std::{
    ffi::OsString,
    fs,
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

#[cfg(not(windows))]
use std::fs::File;

#[cfg(windows)]
pub mod windows_file_stamp;
#[cfg(windows)]
pub mod windows_operations;
#[cfg(windows)]
pub mod windows_paths;

/// Maximum names per listing event, without eager metadata I/O.
pub const LISTING_BATCH_SIZE: usize = 256;
/// Emit the first visible names sooner; subsequent batches stay throughput-oriented.
const FIRST_LISTING_BATCH_SIZE: usize = 32;
/// Prevent unread UI events from consuming unbounded memory.
// Bound stale batches to 8 * 256 names, not 16,384 names.
// New navigation can drain abandoned generations promptly.
const LISTING_CHANNEL_CAPACITY: usize = 8;
// Only the newest selection is useful; cap buffered raw bytes to 2 MiB
// instead of retaining up to 8 MiB of stale 1-MiB previews.
const PREVIEW_CHANNEL_CAPACITY: usize = 2;
/// A raw prefetch bound, not a limit on the final parser-backed preview.
pub const MAX_RAW_PREVIEW_BYTES: usize = 1024 * 1024;

struct MailboxState<T> {
    pending: Option<(u64, T)>,
    stopping: bool,
}

/// One pending request per service; new requests replace stale pending work.
/// An OS call already in flight may finish, but its result cannot be published.
struct Mailbox<T> {
    latest: Arc<AtomicU64>,
    state: Mutex<MailboxState<T>>,
    ready: Condvar,
}

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<T> Mailbox<T> {
    fn new() -> Self {
        Self::with_generation(Arc::new(AtomicU64::new(0)))
    }

    fn with_generation(latest: Arc<AtomicU64>) -> Self {
        Self {
            latest,
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
        if self.is_current(generation)
            && state
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
            if let Some((generation, task)) = state.pending.take() {
                if self.is_current(generation) {
                    return Some((generation, task));
                }
                // A racing submitter must not resurrect superseded work.
                continue;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FastFileKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListingEntry {
    /// Preserve native platform names (including non-Unicode names on Unix).
    pub file_name: OsString,
    /// Windows FindFirst/FindNext supplies type with the enumeration result;
    /// Linux leaves it lazy to avoid a potential stat on unusual filesystems.
    pub fast_kind: Option<FastFileKind>,
    /// Windows hidden attribute from the already-fetched directory entry.
    /// None on Unix, where asking for metadata could cause an extra stat.
    pub hidden_hint: Option<bool>,
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
    #[cfg(windows)]
    remote_mailbox: Arc<Mailbox<PathBuf>>,
    updates: Receiver<ListingEvent>,
}

impl ListingService {
    pub fn new() -> Self {
        let mailbox: Arc<Mailbox<PathBuf>> = Arc::new(Mailbox::new());
        #[cfg(windows)]
        let remote_mailbox: Arc<Mailbox<PathBuf>> =
            Arc::new(Mailbox::with_generation(Arc::clone(&mailbox.latest)));
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
        #[cfg(windows)]
        {
            // UNC paths get a distinct, bounded worker. A stalled remote
            // filesystem call must not consume either of the two local workers.
            // Mapped drives cannot be recognized lexically and still require
            // an OS-specific volume classification in a later phase.
            let worker = Arc::clone(&remote_mailbox);
            thread::Builder::new()
                .name("strata-listing-unc".into())
                .spawn(move || {
                    while let Some((generation, path)) = worker.next() {
                        run_listing(&path, generation, &worker, &sender);
                    }
                })
                .expect("failed to start UNC listing worker");
        }
        Self {
            mailbox,
            #[cfg(windows)]
            remote_mailbox,
            updates,
        }
    }

    /// Return immediately without probing existence, permissions or metadata.
    pub fn request(&self, path: impl Into<PathBuf>) -> u64 {
        let path = path.into();
        #[cfg(windows)]
        if matches!(
            windows_paths::classify(&path),
            windows_paths::PathKind::NetworkAbsolute
        ) {
            return self.remote_mailbox.submit(path);
        }
        self.mailbox.submit(path)
    }

    pub fn try_recv(&self) -> Result<ListingEvent, TryRecvError> {
        // Never surface a stale folder batch to callers. Drain bounded old
        // events so the current navigation does not sit behind them.
        loop {
            let event = self.updates.try_recv()?;
            let generation = match &event {
                ListingEvent::Batch { generation, .. }
                | ListingEvent::Finished { generation, .. } => *generation,
            };
            if self.mailbox.is_current(generation) {
                return Ok(event);
            }
        }
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
        #[cfg(windows)]
        self.remote_mailbox.shutdown();
    }
}

fn run_listing(
    path: &Path,
    generation: u64,
    mailbox: &Mailbox<PathBuf>,
    sender: &SyncSender<ListingEvent>,
) {
    // Do not open a stale, potentially unresponsive UNC path if superseded
    // before this worker started processing it.
    if !mailbox.is_current(generation) {
        return;
    }
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
    let mut batch = Vec::with_capacity(FIRST_LISTING_BATCH_SIZE);
    let mut flush_at = FIRST_LISTING_BATCH_SIZE;
    for entry in listing {
        if !mailbox.is_current(generation) {
            return;
        }
        match entry {
            Ok(entry) => {
                // Avoid per-item stat, thumbnail, icon lookup or ACL probes.
                #[cfg(windows)]
                let fast_kind = entry.file_type().ok().map(|file_type| {
                    if file_type.is_dir() {
                        FastFileKind::Directory
                    } else if file_type.is_file() {
                        FastFileKind::File
                    } else if file_type.is_symlink() {
                        FastFileKind::Symlink
                    } else {
                        FastFileKind::Other
                    }
                });
                #[cfg(not(windows))]
                let fast_kind = None;
                #[cfg(windows)]
                let hidden_hint = {
                    use std::os::windows::fs::MetadataExt;
                    // std::fs::DirEntry::metadata() on Windows reuses data
                    // from FindFirstFile / FindNextFile: no additional syscall.
                    entry
                        .metadata()
                        .ok()
                        .map(|metadata| metadata.file_attributes() & 0x0000_0002 != 0)
                };
                #[cfg(not(windows))]
                let hidden_hint = None;
                batch.push(ListingEntry {
                    file_name: entry.file_name(),
                    fast_kind,
                    hidden_hint,
                });
                if batch.len() == flush_at
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
                // Only the first batch is small. Keep all later sends at 256 names.
                if batch.is_empty() {
                    flush_at = LISTING_BATCH_SIZE;
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
    #[cfg(windows)]
    remote_mailbox: Arc<Mailbox<PreviewTask>>,
    updates: Receiver<PreviewEvent>,
}

impl PreviewService {
    pub fn new() -> Self {
        let mailbox: Arc<Mailbox<PreviewTask>> = Arc::new(Mailbox::new());
        #[cfg(windows)]
        let remote_mailbox: Arc<Mailbox<PreviewTask>> =
            Arc::new(Mailbox::with_generation(Arc::clone(&mailbox.latest)));
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
        #[cfg(windows)]
        {
            let worker = Arc::clone(&remote_mailbox);
            thread::Builder::new()
                .name("strata-preview-unc".into())
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
                .expect("failed to start UNC preview worker");
        }
        Self {
            mailbox,
            #[cfg(windows)]
            remote_mailbox,
            updates,
        }
    }

    /// Latest request wins. Zero-length requests are permitted.
    pub fn request(&self, path: impl Into<PathBuf>, max_bytes: usize) -> u64 {
        let task = PreviewTask {
            path: path.into(),
            max_bytes: max_bytes.min(MAX_RAW_PREVIEW_BYTES),
        };
        #[cfg(windows)]
        if matches!(
            windows_paths::classify(&task.path),
            windows_paths::PathKind::NetworkAbsolute
        ) {
            return self.remote_mailbox.submit(task);
        }
        self.mailbox.submit(task)
    }

    pub fn try_recv(&self) -> Result<PreviewEvent, TryRecvError> {
        loop {
            let event = self.updates.try_recv()?;
            if self.mailbox.is_current(event.generation) {
                return Ok(event);
            }
        }
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
        #[cfg(windows)]
        self.remote_mailbox.shutdown();
    }
}

fn load_preview(
    task: &PreviewTask,
    mailbox: &Mailbox<PreviewTask>,
    generation: u64,
) -> Option<io::Result<RawPreview>> {
    let result = (|| {
        // A superseded request must not open a file or touch a network share.
        if !mailbox.is_current(generation) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "superseded"));
        }
        #[cfg(windows)]
        let mut file = windows_operations::open_regular_source(&task.path)?;
        #[cfg(not(windows))]
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
        // Avoid even one extra network read after a selection changed.
        if !mailbox.is_current(generation) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "superseded"));
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
        let mut batch_sizes = Vec::new();
        wait_for(|| match service.try_recv() {
            Ok(ListingEvent::Batch {
                generation: id,
                entries,
            }) if id == generation => {
                assert!(entries.len() <= LISTING_BATCH_SIZE);
                batch_sizes.push(entries.len());
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
        assert_eq!(
            batch_sizes,
            [
                FIRST_LISTING_BATCH_SIZE,
                LISTING_BATCH_SIZE,
                LISTING_BATCH_SIZE,
                56,
            ]
        );
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

    #[cfg(windows)]
    #[test]
    fn windows_listing_reuses_hidden_attribute_without_extra_stat() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("ordinary.txt"), b"x").expect("create entry");
        let service = ListingService::new();
        let generation = service.request(&fixture.0);
        let entries = wait_for(|| match service.try_recv() {
            Ok(ListingEvent::Batch {
                generation: current,
                entries,
            }) if current == generation => Some(entries),
            _ => None,
        });
        let file = entries
            .iter()
            .find(|entry| entry.file_name == "ordinary.txt")
            .expect("ordinary entry");
        assert_eq!(file.fast_kind, Some(FastFileKind::File));
        assert_eq!(file.hidden_hint, Some(false));
    }

    #[test]
    fn listing_drops_queued_events_from_superseded_folders() {
        // Deterministic: inject stale events, as if a previous scan filled
        // the channel, then navigate without allowing another worker to race.
        let mailbox: Arc<Mailbox<PathBuf>> = Arc::new(Mailbox::new());
        let (sender, updates) = mpsc::sync_channel(LISTING_CHANNEL_CAPACITY);
        let service = ListingService {
            mailbox: Arc::clone(&mailbox),
            #[cfg(windows)]
            remote_mailbox: Arc::new(Mailbox::with_generation(Arc::clone(&mailbox.latest))),
            updates,
        };
        let old = mailbox.submit(PathBuf::from("old-folder"));
        sender
            .try_send(ListingEvent::Batch {
                generation: old,
                entries: vec![],
            })
            .expect("queue stale batch");
        sender
            .try_send(ListingEvent::Finished {
                generation: old,
                error: Some("obsolete failure".into()),
            })
            .expect("queue stale finish");
        let current = mailbox.submit(PathBuf::from("new-folder"));
        sender
            .try_send(ListingEvent::Finished {
                generation: current,
                error: None,
            })
            .expect("queue current finish");
        assert!(matches!(
            service.try_recv(),
            Ok(ListingEvent::Finished {
                generation,
                error: None
            }) if generation == current
        ));
        assert!(matches!(service.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn preview_drops_queued_bytes_from_superseded_selection() {
        let mailbox: Arc<Mailbox<PreviewTask>> = Arc::new(Mailbox::new());
        let (sender, updates) = mpsc::sync_channel(PREVIEW_CHANNEL_CAPACITY);
        let service = PreviewService {
            mailbox: Arc::clone(&mailbox),
            #[cfg(windows)]
            remote_mailbox: Arc::new(Mailbox::with_generation(Arc::clone(&mailbox.latest))),
            updates,
        };
        let old = mailbox.submit(PreviewTask {
            path: PathBuf::from("old"),
            max_bytes: 1,
        });
        sender
            .try_send(PreviewEvent {
                generation: old,
                result: Ok(RawPreview {
                    bytes: b"old".to_vec(),
                    truncated: false,
                }),
            })
            .expect("queue stale preview");
        let current = mailbox.submit(PreviewTask {
            path: PathBuf::from("new"),
            max_bytes: 1,
        });
        sender
            .try_send(PreviewEvent {
                generation: current,
                result: Ok(RawPreview {
                    bytes: b"new".to_vec(),
                    truncated: false,
                }),
            })
            .expect("queue newest preview");
        let event = service.try_recv().expect("must deliver current preview");
        assert_eq!(event.generation, current);
        assert_eq!(event.result.expect("preview").bytes, b"new");
        assert!(matches!(service.try_recv(), Err(TryRecvError::Empty)));
    }

    #[cfg(windows)]
    #[test]
    fn local_and_unc_workers_share_the_latest_generation() {
        let local: Mailbox<PathBuf> = Mailbox::new();
        let remote: Mailbox<PathBuf> = Mailbox::with_generation(Arc::clone(&local.latest));
        let prior = remote.submit(PathBuf::from(r"\\server\share\stalled"));
        let current = local.submit(PathBuf::from(r"C:\fast-local"));
        assert!(current > prior);
        assert!(!remote.is_current(prior));
        assert!(local.is_current(current));
        assert_eq!(
            lock(&remote.state).pending.as_ref().map(|task| task.0),
            Some(prior)
        );
    }

    #[test]
    fn shutdown_stops_pending_mailbox_dispatch() {
        let mailbox = Mailbox::new();
        mailbox.submit(PathBuf::from("before-shutdown"));
        mailbox.shutdown();
        assert!(mailbox.next().is_none());
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
