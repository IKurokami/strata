// SPDX-License-Identifier: MIT

//! Filename search remains independent. Content search and inference have
//! separate latest-request mailboxes and never execute on the GTK context.

use arc_swap::{ArcSwap, ArcSwapOption};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

pub(crate) mod external;
mod hybrid;
mod indexer;
mod media;
mod model;
pub(crate) mod policy;
pub(crate) mod result;
mod runtime;
mod store;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Config {
    pub enabled: bool,
    pub roots: Vec<PathBuf>,
    pub include_code: bool,
    pub index_on_battery: bool,
    pub paused: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            roots: Vec::new(),
            include_code: false,
            index_on_battery: false,
            paused: false,
        }
    }
}

pub(crate) struct Status {
    pub models_ready: bool,
    pub downloading: bool,
    pub downloaded: u64,
    pub total: u64,
    pub indexed: usize,
    pub running: bool,
    pub failed: bool,
    pub folder_counts: HashMap<PathBuf, usize>,
}

impl Status {
    pub fn folder_indexed_count(&self, folder: &Path) -> usize {
        if let Some(&count) = self.folder_counts.get(folder) {
            return count;
        }
        let normalized = policy::normalize_path(
            &std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf()),
        );
        self.folder_counts.get(&normalized).copied().unwrap_or(0)
    }
}

pub(super) struct Control {
    config: ArcSwap<Config>,
    revision: AtomicU64,
    database_revision: AtomicU64,
    interaction: AtomicU64,
    parsing: Mutex<Option<crate::sandbox::Cancellation>>,
    indexer_started: AtomicBool,
    reset: AtomicBool,
    downloading: AtomicBool,
    download_cancel: AtomicBool,
    downloaded: AtomicU64,
    models_ready: AtomicBool,
    model_probe: AtomicBool,
    model_probe_at: AtomicU64,
    indexed: AtomicUsize,
    skipped: AtomicUsize,
    folder_counts: ArcSwap<HashMap<PathBuf, usize>>,
    indexing: AtomicBool,
    failed: AtomicBool,
    data: PathBuf,
}

fn clock() -> u64 {
    static CLOCK: OnceLock<Instant> = OnceLock::new();
    CLOCK
        .get_or_init(Instant::now)
        .elapsed()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn control() -> &'static Arc<Control> {
    static CONTROL: OnceLock<Arc<Control>> = OnceLock::new();
    CONTROL.get_or_init(|| {
        Arc::new(Control {
            config: ArcSwap::from_pointee(Config::default()),
            revision: AtomicU64::new(1),
            database_revision: AtomicU64::new(1),
            interaction: AtomicU64::new(0),
            parsing: Mutex::new(None),
            indexer_started: AtomicBool::new(false),
            reset: AtomicBool::new(false),
            downloading: AtomicBool::new(false),
            download_cancel: AtomicBool::new(false),
            downloaded: AtomicU64::new(0),
            models_ready: AtomicBool::new(false),
            model_probe: AtomicBool::new(false),
            model_probe_at: AtomicU64::new(0),
            indexed: AtomicUsize::new(0),
            skipped: AtomicUsize::new(0),
            folder_counts: ArcSwap::from_pointee(HashMap::new()),
            indexing: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            data: glib::user_cache_dir()
                .join("strata")
                .join("smart-search-v1"),
        })
    })
}

pub(crate) fn configure(config: Config) {
    let shared = control();
    if **shared.config.load() == config {
        return;
    }
    if !config.enabled {
        shared.download_cancel.store(true, Ordering::Release);
    }
    shared.config.store(Arc::new(config));
    shared.revision.fetch_add(1, Ordering::AcqRel);
    note_interaction();
    ensure_indexer();
}

fn ensure_indexer() {
    let shared = control();
    if shared.config.load().roots.is_empty() && !shared.reset.load(Ordering::Acquire) {
        return;
    }
    if !shared.indexer_started.swap(true, Ordering::AcqRel) {
        let worker = shared.clone();
        if std::thread::Builder::new()
            .name("content-indexer".into())
            .spawn(move || indexer::run(worker))
            .is_err()
        {
            shared.indexer_started.store(false, Ordering::Release);
            shared.failed.store(true, Ordering::Release);
        }
    }
}

pub(crate) fn note_interaction() {
    let shared = control();
    shared.interaction.store(clock(), Ordering::Release);
}

pub(crate) fn status() -> Status {
    let shared = control();
    ensure_indexer();
    if !shared.downloading.load(Ordering::Acquire)
        && clock().saturating_sub(shared.model_probe_at.load(Ordering::Relaxed)) >= 5000
        && !shared.model_probe.swap(true, Ordering::AcqRel)
    {
        shared.model_probe_at.store(clock(), Ordering::Relaxed);
        let probe = shared.clone();
        if std::thread::Builder::new()
            .name("search-model-status".into())
            .spawn(move || {
                probe
                    .models_ready
                    .store(model::ready(&probe.data), Ordering::Release);
                probe.model_probe.store(false, Ordering::Release);
            })
            .is_err()
        {
            shared.model_probe.store(false, Ordering::Release);
        }
    }
    Status {
        models_ready: shared.models_ready.load(Ordering::Acquire),
        downloading: shared.downloading.load(Ordering::Acquire),
        downloaded: shared.downloaded.load(Ordering::Relaxed),
        total: model::TOTAL_BYTES,
        indexed: shared.indexed.load(Ordering::Relaxed),
        running: shared.indexing.load(Ordering::Acquire),
        failed: shared.failed.load(Ordering::Acquire),
        folder_counts: (**shared.folder_counts.load()).clone(),
    }
}

pub(crate) fn download_models() {
    let shared = control();
    if !shared.config.load().enabled || shared.downloading.swap(true, Ordering::AcqRel) {
        return;
    }
    shared.download_cancel.store(false, Ordering::Release);
    shared.failed.store(false, Ordering::Release);
    let shared = shared.clone();
    let _ = std::thread::Builder::new()
        .name("search-model-download".into())
        .spawn(move || {
            let result = model::download(&shared.data, &shared.download_cancel, |bytes| {
                shared.downloaded.store(bytes, Ordering::Relaxed)
            });
            shared.models_ready.store(result.is_ok(), Ordering::Release);
            if let Err(error) = result {
                tracing::warn!(reason=%error,"Smart Search model setup stopped");
                shared.failed.store(true, Ordering::Release);
            }
            shared.downloading.store(false, Ordering::Release);
            shared.database_revision.fetch_add(1, Ordering::AcqRel);
            shared.revision.fetch_add(1, Ordering::AcqRel);
            ensure_indexer();
        });
}

/// Called only after the UI's explicit confirmation. Source files and models
/// are outside the set of database filenames the indexer may remove.
pub(crate) fn rebuild_index() {
    let shared = control();
    shared.reset.store(true, Ordering::Release);
    shared.revision.fetch_add(1, Ordering::AcqRel);
    note_interaction();
    ensure_indexer();
}

#[derive(Clone)]
struct Request {
    generation: u64,
    configuration: u64,
    query: String,
    roots: Vec<PathBuf>,
    kind: result::SearchKind,
    submitted: Instant,
}

struct Lane {
    pending: ArcSwapOption<Request>,
    wake: Condvar,
    sleep: Mutex<()>,
}
impl Lane {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            pending: ArcSwapOption::empty(),
            wake: Condvar::new(),
            sleep: Mutex::new(()),
        })
    }
    fn post(&self, request: Arc<Request>) {
        self.pending.store(Some(request));
        self.wake.notify_one();
    }
    fn next(&self, stop: &AtomicBool) -> Option<Arc<Request>> {
        if stop.load(Ordering::Acquire) {
            return None;
        }
        if let Some(request) = self.pending.swap(None) {
            return Some(request);
        }
        let guard = self.sleep.lock().unwrap_or_else(|error| error.into_inner());
        drop(self.wake.wait_timeout(guard, Duration::from_millis(100)));
        self.pending.swap(None)
    }
}

struct SessionState {
    generation: AtomicU64,
    stop: AtomicBool,
    keyword: Arc<Lane>,
    semantic: Arc<Lane>,
    responses: Mutex<Vec<result::SearchResponse>>,
}
pub(crate) struct SearchSession {
    state: Arc<SessionState>,
}

impl SearchSession {
    pub fn new() -> Self {
        let state = Arc::new(SessionState {
            generation: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            keyword: Lane::new(),
            semantic: Lane::new(),
            responses: Mutex::new(Vec::new()),
        });
        for semantic in [false, true] {
            let state = state.clone();
            let _ = std::thread::Builder::new()
                .name(
                    if semantic {
                        "semantic-search"
                    } else {
                        "content-keywords"
                    }
                    .into(),
                )
                .spawn(move || run_lane(state, semantic));
        }
        Self { state }
    }
    pub fn query(
        &self,
        query: &str,
        roots: Vec<PathBuf>,
        mode: usize,
        kind: result::SearchKind,
    ) -> u64 {
        note_interaction();
        let generation = self
            .state
            .generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        if query.trim().is_empty() || roots.is_empty() || mode == 1 {
            return generation;
        }
        let request = Arc::new(Request {
            generation,
            configuration: control().revision.load(Ordering::Acquire),
            query: query.chars().take(1024).collect(),
            roots,
            kind,
            submitted: Instant::now(),
        });
        self.state.keyword.post(request.clone());
        if mode != 2 && control().config.load().enabled {
            self.state.semantic.post(request);
        }
        generation
    }
    pub fn cancel(&self) {
        self.state.generation.fetch_add(1, Ordering::AcqRel);
        self.state.semantic.wake.notify_one();
        self.state.keyword.wake.notify_one();
    }
    pub fn take(&self) -> Vec<result::SearchResponse> {
        let Ok(mut responses) = self.state.responses.try_lock() else {
            return Vec::new();
        };
        let generation = self.state.generation.load(Ordering::Acquire);
        let configuration = control().revision.load(Ordering::Acquire);
        std::mem::take(&mut *responses)
            .into_iter()
            .filter(|response| {
                response.generation == generation && response.configuration == configuration
            })
            .collect()
    }
}
impl Drop for SearchSession {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::Release);
        self.cancel();
    }
}

fn cancelled(state: &SessionState, request: &Request) -> bool {
    state.stop.load(Ordering::Acquire)
        || state.generation.load(Ordering::Acquire) != request.generation
        || control().revision.load(Ordering::Acquire) != request.configuration
        || control().reset.load(Ordering::Acquire)
}

fn keyword(
    data: &Path,
    config: &Config,
    request: &Request,
    state: &Arc<SessionState>,
) -> Vec<result::SearchHit> {
    let mut hits = Vec::new();
    for name in ["index.sqlite", "code.sqlite"] {
        if name == "code.sqlite" && !config.include_code {
            continue;
        }
        if let Ok(mut connection) = store::connect(&data.join(name), false) {
            if interrupt_on_new_request(&mut connection, state, request).is_err() {
                continue;
            }
            if let Ok(rows) = store::keyword(
                &connection,
                &request.query,
                &request.roots,
                config.include_code,
                request.kind,
            ) {
                hits.extend(rows);
            }
        }
    }
    hits.retain(|hit| {
        config
            .roots
            .iter()
            .any(|root| policy::within(&hit.path, root))
    });
    hybrid::distinct_files(hits, 100)
}

fn run_lane(state: Arc<SessionState>, semantic: bool) {
    let shared = control();
    let lane = if semantic {
        state.semantic.clone()
    } else {
        state.keyword.clone()
    };
    let mut embedder: Option<runtime::Embedder> = None;
    let mut vectors: Option<(
        u64,
        u64,
        Vec<PathBuf>,
        result::SearchKind,
        store::VectorIndex,
    )> = None;
    let mut query_cache: std::collections::VecDeque<(String, Vec<f32>, Instant)> =
        std::collections::VecDeque::new();
    let artifacts = tempfile::tempdir().ok();
    while !state.stop.load(Ordering::Acquire) {
        let Some(mut request) = lane.next(&state.stop) else {
            if embedder.as_ref().is_some_and(runtime::Embedder::idle)
                || !shared.config.load().enabled
            {
                embedder = None;
                vectors = None;
                query_cache.clear();
            }
            continue;
        };
        if semantic {
            while request.submitted.elapsed() < Duration::from_millis(120)
                && !state.stop.load(Ordering::Acquire)
            {
                std::thread::sleep(Duration::from_millis(10));
                if let Some(newest) = lane.pending.swap(None) {
                    request = newest;
                }
            }
        }
        if cancelled(&state, &request) {
            continue;
        }
        let config = shared.config.load_full();
        let mut hits = keyword(&shared.data, &config, &request, &state);
        if semantic {
            if !config.enabled || !model::ready(&shared.data) {
                embedder = None;
                shared.models_ready.store(false, Ordering::Release);
                continue;
            }
            shared.models_ready.store(true, Ordering::Release);
            let cancel = || cancelled(&state, &request) || !shared.config.load().enabled;
            let revision = shared.database_revision.load(Ordering::Acquire);
            if vectors
                .as_ref()
                .is_none_or(|(known, configuration, roots, kind, _)| {
                    *known != revision
                        || *configuration != request.configuration
                        || roots != &request.roots
                        || *kind != request.kind
                })
            {
                let mut matrix = store::VectorIndex {
                    hits: Vec::new(),
                    values: Vec::new(),
                };
                for name in ["index.sqlite", "code.sqlite"] {
                    if name == "code.sqlite" && !config.include_code {
                        continue;
                    }
                    if let Ok(mut connection) = store::connect(&shared.data.join(name), false) {
                        if interrupt_on_new_request(&mut connection, &state, &request).is_err() {
                            continue;
                        }
                        if let Ok(index) = store::vectors(
                            &connection,
                            model::FINGERPRINT,
                            &request.roots,
                            config.include_code,
                            request.kind,
                        ) {
                            let remaining = 80_000usize.saturating_sub(matrix.hits.len());
                            matrix.hits.extend(index.hits.into_iter().take(remaining));
                            matrix
                                .values
                                .extend(index.values.into_iter().take(remaining * 768));
                        }
                    }
                }
                vectors = Some((
                    revision,
                    request.configuration,
                    request.roots.clone(),
                    request.kind,
                    matrix,
                ));
            }
            if vectors
                .as_ref()
                .is_none_or(|(_, _, _, _, index)| index.hits.is_empty())
            {
                continue;
            }
            if embedder.is_none() {
                let Some(artifacts) = artifacts.as_ref() else {
                    continue;
                };
                let worker_cancel = || {
                    state.stop.load(Ordering::Acquire)
                        || !shared.config.load().enabled
                        || shared.reset.load(Ordering::Acquire)
                };
                embedder = match runtime::Embedder::start(
                    &shared.data,
                    artifacts.path(),
                    true,
                    &worker_cancel,
                ) {
                    Ok(worker) => {
                        shared.failed.store(false, Ordering::Release);
                        Some(worker)
                    }
                    Err(error) => {
                        if !worker_cancel() {
                            shared.failed.store(true, Ordering::Release);
                        }
                        tracing::debug!(reason=%error,"Semantic query runtime unavailable");
                        None
                    }
                };
                if cancelled(&state, &request) {
                    continue;
                }
            }
            let Some(worker) = embedder.as_mut() else {
                continue;
            };
            query_cache.retain(|(_, _, when)| when.elapsed() < Duration::from_secs(300));
            let cache_key = format!("{}:{}", config.include_code, request.query);
            let cached = query_cache
                .iter()
                .find(|(query, _, _)| query == &cache_key)
                .map(|(_, vector, _)| vector.clone());
            let embedding = if let Some(cached) = cached {
                Some(cached)
            } else {
                let mut prompts = vec![format!("task: search result | query: {}", request.query)];
                if config.include_code {
                    prompts.push(format!("task: code retrieval | query: {}", request.query));
                }
                match worker.embed("text", &prompts, &cancel) {
                    Ok(rows) => {
                        let mut rows = rows.into_iter();
                        rows.next().flatten().map(|mut normal| {
                            if let Some(Some(code)) = rows.next() {
                                normal.extend(code);
                            }
                            normal
                        })
                    }
                    Err(_) => {
                        embedder = None;
                        None
                    }
                }
            };
            let Some(embedding) = embedding else { continue };
            if !query_cache.iter().any(|(query, _, _)| query == &cache_key) {
                query_cache.push_back((cache_key, embedding.clone(), Instant::now()));
                if query_cache.len() > 256 {
                    query_cache.pop_front();
                }
            }
            if let Some((_, _, _, _, index)) = &vectors {
                hits = hybrid::rank(
                    &request.query,
                    &embedding[..768],
                    embedding.get(768..).filter(|vector| vector.len() == 768),
                    index,
                    hits,
                    cancel,
                );
            }
        }
        if cancelled(&state, &request) {
            continue;
        }
        // Only worker lanes validate freshness; GTK never stats a result path.
        hits.retain(|hit| {
            config
                .roots
                .iter()
                .any(|root| policy::within(&hit.path, root))
                && std::fs::metadata(&hit.path).is_ok_and(|metadata| {
                    result::FileRevision::from_metadata(&metadata) == hit.revision
                })
        });
        if cancelled(&state, &request) {
            continue;
        }
        tracing::debug!(
            generation = request.generation,
            semantic,
            elapsed_ms = request.submitted.elapsed().as_millis(),
            count = hits.len(),
            "content search response ready"
        );
        for hit in &mut hits {
            hybrid::locate(&request.query, hit);
        }
        let response = result::SearchResponse {
            generation: request.generation,
            configuration: request.configuration,
            phase: if semantic {
                result::SearchPhase::Semantic
            } else {
                result::SearchPhase::Keyword
            },
            hits,
        };
        let mut responses = state
            .responses
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        responses
            .retain(|old| old.generation == response.generation && old.phase != response.phase);
        responses.push(response);
    }
}

#[cfg(test)]
mod tests;

pub(crate) fn configured_roots() -> Vec<PathBuf> {
    control().config.load().roots.clone()
}

pub(crate) fn cancel_download() {
    control().download_cancel.store(true, Ordering::Release);
}

fn interrupt_on_new_request(
    connection: &mut rusqlite::Connection,
    state: &Arc<SessionState>,
    request: &Request,
) -> Result<(), String> {
    let state = state.clone();
    let generation = request.generation;
    let configuration = request.configuration;
    connection
        .progress_handler(
            1000,
            Some(move || {
                state.stop.load(Ordering::Acquire)
                    || state.generation.load(Ordering::Acquire) != generation
                    || control().revision.load(Ordering::Acquire) != configuration
            }),
        )
        .map_err(|error| error.to_string())
}
