// SPDX-License-Identifier: MIT

use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use async_channel::Sender;
use notify::Watcher;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

const MAX_NODES: usize = 200_000;
const MAX_DIRECTORIES: usize = 4_000;
const MAX_CHILDREN: usize = 300;
const MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum Coverage {
    Exact,
    Partial,
    Stale,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Child {
    pub path: PathBuf,
    pub bytes: u64,
    pub files: u64,
    pub directory: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Snapshot {
    pub root: PathBuf,
    pub children: Vec<Child>,
    pub other_bytes: u64,
    pub other_count: usize,
    pub total: u64,
    #[serde(default)]
    pub files: u64,
    pub coverage: Coverage,
    pub generation: u64,
    #[serde(default)]
    pub identity: Option<String>,
    #[serde(default)]
    pub resumable: bool,
}

enum Message {
    Subscribe(PathBuf, Sender<Snapshot>),
    Changed(Vec<PathBuf>),
    Clear(Sender<std::io::Result<()>>),
    Promote(PathBuf),
}

struct Service {
    requests: mpsc::SyncSender<Message>,
}
static SERVICE: OnceLock<Service> = OnceLock::new();
static OVERFLOW: AtomicBool = AtomicBool::new(false);

fn service() -> &'static Service {
    SERVICE.get_or_init(|| {
        let (requests, events) = mpsc::sync_channel(32);
        let changes = requests.clone();
        let database = gtk::glib::user_cache_dir().join("strata/disk-usage-v1.sqlite");
        std::thread::Builder::new()
            .name("strata-disk-usage".into())
            .spawn(move || run(events, changes, &database))
            .expect("disk usage worker");
        Service { requests }
    })
}

pub(crate) fn subscribe(path: PathBuf) -> async_channel::Receiver<Snapshot> {
    let (sender, receiver) = async_channel::bounded(2);
    if service()
        .requests
        .try_send(Message::Subscribe(path, sender))
        .is_err()
    {
        tracing::warn!("disk usage request queue is full");
    }
    receiver
}

pub(crate) fn promote(path: PathBuf) {
    let _ = service().requests.try_send(Message::Promote(path));
}

pub(crate) fn invalidate(path: PathBuf) {
    if service()
        .requests
        .try_send(Message::Changed(vec![path]))
        .is_err()
    {
        OVERFLOW.store(true, Ordering::Release);
    }
}

pub(crate) async fn clear_cache() -> std::io::Result<()> {
    let (sender, receiver) = async_channel::bounded(1);
    service()
        .requests
        .try_send(Message::Clear(sender))
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::WouldBlock))?;
    receiver
        .recv()
        .await
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::Interrupted))?
}

struct Node {
    path: PathBuf,
    parent: Option<usize>,
    directory: bool,
    bytes: u64,
    files: u64,
    complete: bool,
    unavailable: bool,
    reused: bool,
    identity: Option<String>,
}

struct Scan {
    nodes: Vec<Node>,
    pending: VecDeque<usize>,
    current: Option<(usize, std::iter::Peekable<fs::ReadDir>)>,
    directories: usize,
    started: Instant,
    published: Instant,
    generation: u64,
    retained_bytes: usize,
    #[cfg(unix)]
    device: Option<u64>,
}

impl Scan {
    fn new(path: PathBuf, generation: u64) -> Self {
        Self {
            nodes: vec![Node {
                path: path.clone(),
                parent: None,
                directory: true,
                bytes: 0,
                files: 0,
                complete: true,
                unavailable: false,
                reused: false,
                identity: root_identity(&path),
            }],
            pending: VecDeque::from([0]),
            current: None,
            directories: 1,
            started: Instant::now(),
            published: Instant::now() - Duration::from_millis(200),
            generation,
            retained_bytes: 0,
            #[cfg(unix)]
            device: {
                use std::os::unix::fs::MetadataExt;
                fs::symlink_metadata(&path)
                    .ok()
                    .map(|metadata| metadata.dev())
            },
        }
    }

    fn contains(&self, path: &Path) -> bool {
        path.starts_with(&self.nodes[0].path)
    }

    fn step(&mut self, known: &HashMap<PathBuf, Snapshot>) -> bool {
        if self.nodes.len() >= MAX_NODES
            || self.retained_bytes >= MAX_CACHE_BYTES
            || self.started.elapsed() > Duration::from_secs(120)
        {
            self.nodes[0].complete = false;
            self.pending.clear();
            self.current.take();
        }
        let active = self.current.take();
        let parent = if let Some((parent, _)) = active.as_ref() {
            *parent
        } else if let Some(parent) = self.pending.pop_front() {
            parent
        } else {
            return true;
        };
        if parent == 0
            && fs::symlink_metadata(&self.nodes[0].path).is_ok_and(|metadata| boundary(&metadata))
        {
            self.nodes[0].complete = false;
            self.nodes[0].unavailable = true;
            return true;
        }
        let mut directory = match active {
            Some((_, directory)) => directory,
            None => match fs::read_dir(&self.nodes[parent].path) {
                Ok(directory) => directory.peekable(),
                Err(_) => {
                    self.nodes[parent].complete = false;
                    self.nodes[parent].unavailable = true;
                    return self.pending.is_empty();
                }
            },
        };
        for entry in directory.by_ref().take(512) {
            if self.nodes.len() >= MAX_NODES || self.retained_bytes >= MAX_CACHE_BYTES {
                self.nodes[parent].complete = false;
                break;
            }
            let Ok(entry) = entry else {
                self.nodes[parent].complete = false;
                continue;
            };
            let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
                self.nodes[parent].complete = false;
                continue;
            };
            if boundary(&metadata) || {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    self.device.is_some_and(|device| metadata.dev() != device)
                }
                #[cfg(not(unix))]
                {
                    false
                }
            } {
                self.nodes[parent].complete = false;
                continue;
            }
            self.retained_bytes = self.retained_bytes.saturating_add(
                entry.path().as_os_str().as_encoded_bytes().len() + std::mem::size_of::<Node>(),
            );
            let directory = metadata.is_dir();
            let identity = if directory {
                metadata_identity(&entry.path(), &metadata)
            } else {
                None
            };
            let cached = if directory {
                known.get(&entry.path()).filter(|snapshot| {
                    snapshot.coverage == Coverage::Exact
                        && identity.is_some()
                        && snapshot.identity == identity
                })
            } else {
                None
            };
            let index = self.nodes.len();
            self.nodes.push(Node {
                path: entry.path(),
                parent: Some(parent),
                directory,
                bytes: cached.map_or_else(
                    || if directory { 0 } else { metadata.len() },
                    |snapshot| snapshot.total,
                ),
                files: cached.map_or(u64::from(!directory), |snapshot| snapshot.files),
                complete: true,
                unavailable: false,
                reused: cached.is_some(),
                identity,
            });
            if directory && cached.is_none() {
                if self.directories < MAX_DIRECTORIES {
                    self.directories += 1;
                    self.pending.push_back(index);
                } else {
                    self.nodes[index].complete = false;
                }
            }
        }
        if directory.peek().is_some() {
            if self.nodes.len() < MAX_NODES && self.retained_bytes < MAX_CACHE_BYTES {
                self.current = Some((parent, directory));
            } else {
                self.nodes[parent].complete = false;
            }
        }
        self.pending.is_empty() && self.current.is_none()
    }

    fn snapshots(&self, complete: bool) -> Vec<Snapshot> {
        let mut totals: Vec<_> = self
            .nodes
            .iter()
            .map(|node| (node.bytes, node.files, node.complete))
            .collect();
        for index in (1..self.nodes.len()).rev() {
            if let Some(parent) = self.nodes[index].parent {
                let (bytes, files, valid) = totals[index];
                let (sum, overflow) = totals[parent].0.overflowing_add(bytes);
                totals[parent].0 = if overflow { u64::MAX } else { sum };
                totals[parent].1 = totals[parent].1.saturating_add(files);
                totals[parent].2 &= valid && !overflow;
            }
        }
        let mut children: HashMap<usize, Vec<Child>> = HashMap::new();
        for (index, node) in self.nodes.iter().enumerate().skip(1) {
            children
                .entry(node.parent.expect("child parent"))
                .or_default()
                .push(Child {
                    path: node.path.clone(),
                    bytes: totals[index].0,
                    files: totals[index].1,
                    directory: node.directory,
                });
        }
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.directory && !node.reused)
            .map(|(index, node)| {
                let mut entries = children.remove(&index).unwrap_or_default();
                entries.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
                let other_count = entries.len().saturating_sub(MAX_CHILDREN);
                let other_bytes = entries
                    .iter()
                    .skip(MAX_CHILDREN)
                    .fold(0_u64, |sum, child| sum.saturating_add(child.bytes));
                entries.truncate(MAX_CHILDREN);
                Snapshot {
                    root: node.path.clone(),
                    children: entries,
                    other_count,
                    other_bytes,
                    total: totals[index].0,
                    files: totals[index].1,
                    coverage: if node.unavailable {
                        Coverage::Unavailable
                    } else if complete && totals[index].2 {
                        Coverage::Exact
                    } else {
                        Coverage::Partial
                    },
                    generation: self.generation,
                    identity: node.identity.clone(),
                    resumable: !complete,
                }
            })
            .collect()
    }
}

fn changed_scope(path: &Path, root: &Path) -> bool {
    #[cfg(windows)]
    {
        let path: Vec<_> = path
            .components()
            .map(|part| part.as_os_str().to_string_lossy().to_lowercase())
            .collect();
        let root: Vec<_> = root
            .components()
            .map(|part| part.as_os_str().to_string_lossy().to_lowercase())
            .collect();
        path.starts_with(&root) || root.starts_with(&path)
    }
    #[cfg(not(windows))]
    {
        path.starts_with(root) || root.starts_with(path)
    }
}

fn root_identity(path: &Path) -> Option<String> {
    metadata_identity(path, &fs::symlink_metadata(path).ok()?)
}

fn metadata_identity(path: &Path, metadata: &fs::Metadata) -> Option<String> {
    #[cfg(windows)]
    {
        let _ = metadata;
        let (volume, id) = strata_win_shell::directory_identity(path).ok()?;
        Some(format!("{volume}:{id:02x?}"))
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = path;
        Some(format!(
            "{}:{}:{:?}",
            metadata.dev(),
            metadata.ino(),
            metadata.created().ok()
        ))
    }
}

fn boundary(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Reparse points, offline files and recall-on-access placeholders must not hydrate.
        metadata.file_attributes() & (0x400 | 0x1000 | 0x40000 | 0x400000) != 0
    }
    #[cfg(unix)]
    {
        false
    }
}

fn connect(path: &Path) -> rusqlite::Result<Connection> {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let connection = Connection::open(path)?;
    connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA max_page_count=131072; PRAGMA user_version=1;
        CREATE TABLE IF NOT EXISTS snapshots(root TEXT PRIMARY KEY, data BLOB NOT NULL, used INTEGER NOT NULL);")?;
    Ok(connection)
}

fn run(events: mpsc::Receiver<Message>, changes: mpsc::SyncSender<Message>, database: &Path) {
    let dirty = changes.clone();
    let own_database = database.to_path_buf();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event
            .as_ref()
            .is_ok_and(|event| matches!(event.kind, notify::EventKind::Access(_)))
        {
            return;
        }
        let paths = match event {
            Ok(event) => {
                let paths: Vec<_> = event
                    .paths
                    .into_iter()
                    .filter(|path| {
                        !(path.parent() == own_database.parent()
                            && path.file_name().is_some_and(|name| {
                                name.to_string_lossy().starts_with("disk-usage-v1.sqlite")
                            }))
                    })
                    .collect();
                if paths.is_empty() {
                    return;
                }
                paths
            }
            Err(_) => Vec::new(),
        };
        if dirty.try_send(Message::Changed(paths)).is_err() {
            OVERFLOW.store(true, Ordering::Release);
        }
    })
    .ok();
    let mut watched = VecDeque::<PathBuf>::new();
    let mut store = connect(database).ok();
    let mut cache = HashMap::<PathBuf, Snapshot>::new();
    let mut subscribers = HashMap::<PathBuf, Vec<Sender<Snapshot>>>::new();
    let mut scans = Vec::<Scan>::new();
    let mut generation = 0_u64;
    let mut dirty_roots = HashMap::<PathBuf, Instant>::new();
    loop {
        subscribers.retain(|_, senders| {
            senders.retain(|sender| !sender.is_closed());
            !senders.is_empty()
        });
        let message = events.recv_timeout(if scans.is_empty() {
            Duration::from_millis(250)
        } else {
            Duration::ZERO
        });
        match message {
            Ok(Message::Subscribe(path, sender)) => {
                if sender.is_closed() {
                    continue;
                }
                let identity = root_identity(&path);
                let cached = cache.get(&path).cloned().or_else(|| {
                    store.as_ref().and_then(|db| {
                        let bytes: Option<Vec<u8>> = db
                            .query_row(
                                "SELECT data FROM snapshots WHERE root=?",
                                [path.to_string_lossy().as_ref()],
                                |row| row.get(0),
                            )
                            .optional()
                            .ok()?;
                        bytes
                            .and_then(|bytes| serde_json::from_slice::<Snapshot>(&bytes).ok())
                            .map(|mut snapshot| {
                                snapshot.coverage = Coverage::Stale;
                                snapshot
                            })
                    })
                });
                let cached =
                    cached.filter(|snapshot| identity.is_some() && snapshot.identity == identity);
                if let Some(snapshot) = cached.as_ref() {
                    let _ = sender.force_send(snapshot.clone());
                }
                subscribers.entry(path.clone()).or_default().push(sender);
                if !watched.contains(&path)
                    && let Some(watcher) = watcher.as_mut()
                {
                    if watched.len() == MAX_DIRECTORIES
                        && let Some(old) = watched.pop_front()
                    {
                        let _ = watcher.unwatch(&old);
                    }
                    if watcher
                        .watch(&path, notify::RecursiveMode::NonRecursive)
                        .is_ok()
                    {
                        watched.push_back(path.clone());
                    }
                }
                for scan in &mut scans {
                    if let Some(position) = scan
                        .pending
                        .iter()
                        .position(|index| scan.nodes[*index].path == path)
                        && let Some(index) = scan.pending.remove(position)
                    {
                        scan.pending.push_front(index);
                    }
                }
                if cached.is_none_or(|snapshot| {
                    snapshot.coverage == Coverage::Stale || snapshot.resumable
                }) && !scans.iter().any(|scan| scan.contains(&path))
                {
                    generation = generation.wrapping_add(1);
                    let scan = Scan::new(path.to_path_buf(), generation);
                    if scans
                        .iter()
                        .any(|existing| existing.nodes[0].path.starts_with(&path))
                    {
                        scans.insert(0, scan);
                    } else {
                        scans.push(scan);
                    }
                }
            }
            Ok(Message::Promote(path)) => {
                if let Some(position) = scans.iter().position(|scan| scan.contains(&path)) {
                    let scan = scans.remove(position);
                    scans.push(scan);
                } else if cache.get(&path).is_some_and(|snapshot| {
                    snapshot.resumable || snapshot.coverage == Coverage::Stale
                }) {
                    generation = generation.wrapping_add(1);
                    scans.push(Scan::new(path, generation));
                }
            }
            Ok(Message::Clear(done)) => {
                scans.clear();
                dirty_roots.clear();
                cache.clear();
                generation = generation.wrapping_add(1);
                let result = store
                    .as_mut()
                    .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotConnected))
                    .and_then(|db| {
                        db.execute_batch(
                            "DELETE FROM snapshots; PRAGMA wal_checkpoint(TRUNCATE); VACUUM;",
                        )
                        .map_err(std::io::Error::other)
                    });
                let _ = done.force_send(result);
                for path in subscribers.keys() {
                    scans.push(Scan::new(path.to_path_buf(), generation));
                }
            }
            Ok(Message::Changed(paths)) => {
                if paths.is_empty() {
                    if let Some(watcher) = watcher.as_mut() {
                        for path in watched.drain(..) {
                            let _ = watcher.unwatch(&path);
                        }
                    }
                }
                for (root, snapshot) in &mut cache {
                    if paths.is_empty() || paths.iter().any(|path| changed_scope(path, root)) {
                        snapshot.coverage = Coverage::Stale;
                    }
                }
                // Persisted snapshots are treated as stale when read, and active roots
                // reconcile once after a quiet interval rather than on every write.
                for path in subscribers.keys().filter(|root| {
                    paths.is_empty() || paths.iter().any(|path| changed_scope(path, root))
                }) {
                    dirty_roots.insert(path.clone(), Instant::now() + Duration::from_millis(500));
                    scans.retain(|scan| {
                        !scan.contains(path) && !path.starts_with(&scan.nodes[0].path)
                    });
                    if let Some(snapshot) = cache.get(path) {
                        if let Some(senders) = subscribers.get(path) {
                            for sender in senders {
                                let _ = sender.force_send(snapshot.clone());
                            }
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        let ready: Vec<_> = dirty_roots
            .iter()
            .filter(|(_, deadline)| **deadline <= Instant::now())
            .map(|(path, _)| path.clone())
            .collect();
        for path in ready {
            dirty_roots.remove(&path);
            if subscribers.contains_key(&path) && !scans.iter().any(|scan| scan.contains(&path)) {
                generation = generation.wrapping_add(1);
                scans.push(Scan::new(path, generation));
            }
        }
        if OVERFLOW.swap(false, Ordering::AcqRel) {
            for snapshot in cache.values_mut() {
                snapshot.coverage = Coverage::Stale;
            }
            for path in subscribers.keys() {
                if !scans.iter().any(|scan| scan.contains(path)) {
                    generation = generation.wrapping_add(1);
                    scans.push(Scan::new(path.to_path_buf(), generation));
                }
            }
        }
        while scans.len() > 4
            || scans.len() > 1
                && scans.iter().map(|scan| scan.retained_bytes).sum::<usize>() > MAX_CACHE_BYTES
        {
            let evicted = scans.remove(0);
            for snapshot in evicted.snapshots(false) {
                if let Some(senders) = subscribers.get(&snapshot.root) {
                    for sender in senders {
                        let _ = sender.force_send(snapshot.clone());
                    }
                    cache.insert(snapshot.root.clone(), snapshot);
                }
            }
        }
        let Some(scan) = scans.last_mut() else {
            continue;
        };
        let complete = scan.step(&cache);
        if let Some(directory) = scan
            .pending
            .front()
            .map(|index| scan.nodes[*index].path.clone())
            && !watched.contains(&directory)
            && watched.len() < MAX_DIRECTORIES
            && let Some(watcher) = watcher.as_mut()
            && watcher
                .watch(&directory, notify::RecursiveMode::NonRecursive)
                .is_ok()
        {
            watched.push_back(directory);
        }
        if complete || scan.published.elapsed() >= Duration::from_millis(200) {
            let mut transaction = if complete {
                store.as_mut().and_then(|db| db.transaction().ok())
            } else {
                None
            };
            for snapshot in scan.snapshots(complete) {
                if let Some(senders) = subscribers.get(&snapshot.root) {
                    for sender in senders {
                        let _ = sender.force_send(snapshot.clone());
                    }
                }
                if complete {
                    if let (Some(db), Ok(bytes)) =
                        (transaction.as_mut(), serde_json::to_vec(&snapshot))
                    {
                        let _ = db.execute(
                            "INSERT OR REPLACE INTO snapshots VALUES(?,?,unixepoch())",
                            params![snapshot.root.to_string_lossy(), bytes],
                        );
                    }
                    cache.insert(snapshot.root.clone(), snapshot);
                }
            }
            if let Some(transaction) = transaction {
                let _ = transaction.commit();
            }
            scan.published = Instant::now();
        }
        if complete {
            scans.pop();
            let estimate: usize = cache
                .values()
                .map(|snapshot| {
                    snapshot
                        .children
                        .iter()
                        .map(|child| {
                            child.path.as_os_str().as_encoded_bytes().len()
                                + std::mem::size_of::<Child>()
                        })
                        .sum::<usize>()
                        + 256
                })
                .sum();
            if estimate > MAX_CACHE_BYTES {
                cache.retain(|path, _| subscribers.contains_key(path));
            }
            if let Some(db) = store.as_mut() {
                let _ = db.execute("DELETE FROM snapshots WHERE root IN (SELECT root FROM snapshots ORDER BY used DESC LIMIT -1 OFFSET 4000)", []);
            }
        }
    }
}

#[cfg(test)]
mod tests;
