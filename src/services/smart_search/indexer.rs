// SPDX-License-Identifier: MIT

use super::{
    Config, Control, clock, model, policy,
    result::{ContentLocator, ExtractedSegment, Extraction, FileRevision},
    runtime::Embedder,
    store,
};
use notify::{RecursiveMode, Watcher};
use std::{
    collections::{HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering, mpsc},
    time::{Duration, Instant},
};

const MAX_FILES: usize = 200_000;
const MAX_DATABASE_BYTES: u64 = 512 * 1024 * 1024;

pub(super) fn run(shared: Arc<Control>) {
    let artifacts = match tempfile::tempdir() {
        Ok(directory) => directory,
        Err(_) => return,
    };
    let (send, events) = mpsc::sync_channel(64);
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let _ = send.try_send(event);
    })
    .ok();
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut revision = 0;
    let mut walks: VecDeque<(PathBuf, ignore::Walk)> = VecDeque::new();
    let mut urgent: VecDeque<PathBuf> = VecDeque::new();
    let mut backfill: VecDeque<PathBuf> = VecDeque::new();
    let mut seen = HashSet::new();
    let mut complete = Vec::new();
    let mut incomplete = HashSet::new();
    let mut regular: Option<rusqlite::Connection> = None;
    let mut code: Option<rusqlite::Connection> = None;
    let mut embedder = None;
    let mut reconcile = Instant::now();
    loop {
        let config = shared.config.load_full();
        if shared.reset.load(Ordering::Acquire) {
            regular = None;
            code = None;
            embedder = None;
            let mut failed = false;
            for name in [
                "index.sqlite",
                "code.sqlite",
                "index.sqlite-wal",
                "index.sqlite-shm",
                "code.sqlite-wal",
                "code.sqlite-shm",
            ] {
                if let Err(error) = fs::remove_file(shared.data.join(name))
                    && error.kind() != std::io::ErrorKind::NotFound
                {
                    failed = true;
                }
            }
            if failed {
                shared.failed.store(true, Ordering::Release);
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            shared.indexed.store(0, Ordering::Relaxed);
            shared.skipped.store(0, Ordering::Relaxed);
            shared.reset.store(false, Ordering::Release);
            shared.database_revision.fetch_add(1, Ordering::AcqRel);
            revision = 0;
        }
        if !config.enabled || config.paused || embedder.as_ref().is_some_and(Embedder::idle) {
            embedder = None;
        }
        let current = shared.revision.load(Ordering::Acquire);
        if revision != current {
            for root in &roots {
                if let Some(watcher) = watcher.as_mut() {
                    let _ = watcher.unwatch(root);
                }
            }
            roots = config
                .roots
                .iter()
                .map(|root| fs::canonicalize(root).unwrap_or_else(|_| root.clone()))
                .filter(|root| !policy::sensitive(root))
                .collect();
            for root in &roots {
                if let Some(watcher) = watcher.as_mut() {
                    let _ = watcher.watch(root, RecursiveMode::Recursive);
                }
            }
            walks = roots
                .iter()
                .map(|root| (root.clone(), walk(root)))
                .collect();
            seen.clear();
            complete.clear();
            incomplete.clear();
            urgent.clear();
            backfill.clear();
            revision = current;
            regular = store::connect(&shared.data.join("index.sqlite"), true).ok();
            code = store::connect(&shared.data.join("code.sqlite"), true).ok();
            for connection in [&regular, &code].into_iter().flatten() {
                let _ = store::reconcile(connection, &roots, &HashSet::new(), &[]);
            }
            shared
                .models_ready
                .store(model::ready(&shared.data), Ordering::Release);
            embedder = None;
        }
        while let Ok(event) = events.try_recv() {
            if let Ok(event) = event {
                for path in event.paths {
                    if urgent.len() < 1024 {
                        urgent.push_front(path);
                    }
                }
            }
        }
        let quiet = clock().saturating_sub(shared.interaction.load(Ordering::Acquire)) >= 750;
        if config.paused || !quiet || roots.is_empty() {
            shared.indexing.store(false, Ordering::Release);
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        if !config.index_on_battery && on_battery() {
            shared.indexing.store(false, Ordering::Release);
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }
        let database_bytes = [
            "index.sqlite",
            "code.sqlite",
            "index.sqlite-wal",
            "code.sqlite-wal",
        ]
        .iter()
        .filter_map(|name| fs::metadata(shared.data.join(name)).ok())
        .map(|metadata| metadata.len())
        .sum::<u64>();
        if database_bytes > MAX_DATABASE_BYTES {
            shared.failed.store(true, Ordering::Release);
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }
        let next = if let Some(path) = urgent.pop_front() {
            roots
                .iter()
                .find(|root| policy::within(&path, root))
                .cloned()
                .map(|root| (root, path))
        } else if let Some((root, mut scan)) = walks.pop_front() {
            match scan.next() {
                Some(Ok(entry)) => {
                    let path = entry.into_path();
                    walks.push_back((root.clone(), scan));
                    Some((root, path))
                }
                Some(Err(_)) => {
                    incomplete.insert(root.clone());
                    walks.push_back((root, scan));
                    None
                }
                None => {
                    if !incomplete.contains(&root) {
                        complete.push(root);
                    }
                    None
                }
            }
        } else {
            backfill.pop_front().and_then(|path| {
                roots
                    .iter()
                    .find(|root| policy::within(&path, root))
                    .cloned()
                    .map(|root| (root, path))
            })
        };
        let Some((root, path)) = next else {
            shared.indexing.store(false, Ordering::Release);
            if !complete.is_empty() {
                for connection in [&regular, &code].into_iter().flatten() {
                    let _ = store::reconcile(connection, &roots, &seen, &complete);
                    let _ = connection.execute_batch("PRAGMA wal_checkpoint(PASSIVE)");
                }
                complete.clear();
                let count = |state: &str| {
                    [&regular, &code]
                        .into_iter()
                        .flatten()
                        .filter_map(|connection| {
                            connection
                                .query_row(
                                    "SELECT count(*) FROM files WHERE state=?1",
                                    [state],
                                    |row| row.get::<_, i64>(0).map(|count| count.max(0) as usize),
                                )
                                .ok()
                        })
                        .sum()
                };
                shared.indexed.store(count("indexed"), Ordering::Relaxed);
                shared.skipped.store(count("skipped"), Ordering::Relaxed);
                shared.database_revision.fetch_add(1, Ordering::AcqRel);
            }
            if reconcile.elapsed() > Duration::from_secs(120) {
                walks = roots
                    .iter()
                    .map(|root| (root.clone(), walk(root)))
                    .collect();
                seen.clear();
                incomplete.clear();
                reconcile = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(100));
            continue;
        };
        if seen.len() >= MAX_FILES {
            walks.clear();
            complete.clear();
            shared.failed.store(true, Ordering::Release);
            continue;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                if error.kind() == std::io::ErrorKind::NotFound && root.is_dir() {
                    for connection in [&regular, &code].into_iter().flatten() {
                        let _ = connection.execute(
                            "DELETE FROM files WHERE path=?1",
                            [path.to_string_lossy().as_ref()],
                        );
                    }
                    shared.database_revision.fetch_add(1, Ordering::AcqRel);
                }
                if error.kind() != std::io::ErrorKind::NotFound {
                    incomplete.insert(root.clone());
                }
                continue;
            }
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            continue;
        }
        seen.insert(path.clone());
        let is_code = policy::code(&path);
        if policy::sensitive(&path)
            || path.strip_prefix(&root).is_ok_and(policy::excluded)
            || policy::cloud_placeholder(&metadata)
            || (is_code && !config.include_code)
        {
            continue;
        }
        let file_revision = FileRevision::from_metadata(&metadata);
        let connection = if is_code {
            code.as_mut()
        } else {
            regular.as_mut()
        };
        let Some(connection) = connection else {
            continue;
        };
        let fingerprint = (config.enabled && shared.models_ready.load(Ordering::Acquire))
            .then_some(model::FINGERPRINT);
        if store::current(connection, &path, &file_revision, fingerprint).unwrap_or(false) {
            continue;
        }
        shared.indexing.store(true, Ordering::Release);
        let cancelled = || {
            shared.revision.load(Ordering::Acquire) != revision
                || shared.reset.load(Ordering::Acquire)
                || clock().saturating_sub(shared.interaction.load(Ordering::Acquire)) < 750
        };
        let cancellation = crate::sandbox::Cancellation::default();
        *shared
            .parsing
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(cancellation.clone());
        let outcome = index_file(
            &path,
            &file_revision,
            &config,
            connection,
            &root,
            &shared,
            &mut embedder,
            artifacts.path(),
            &cancellation,
            &cancelled,
        );
        shared
            .parsing
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        match outcome {
            Ok(next) => {
                shared.indexed.fetch_add(1, Ordering::Relaxed);
                shared.database_revision.fetch_add(1, Ordering::AcqRel);
                if next && backfill.len() < MAX_FILES {
                    backfill.push_back(path);
                }
            }
            Err(_) if cancelled() || cancellation.is_cancelled() => {
                embedder = None;
                if urgent.len() < 1024 {
                    urgent.push_front(path);
                }
            }
            Err(error) => {
                embedder = None;
                let _ = store::mark(connection, &root, &path, &file_revision, &error);
                shared.skipped.fetch_add(1, Ordering::Relaxed);
                tracing::debug!(reason=%error,"content indexing skipped one file");
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn walk(root: &Path) -> ignore::Walk {
    let root = root.to_owned();
    let mut builder = ignore::WalkBuilder::new(&root);
    builder
        .hidden(false)
        .follow_links(false)
        .git_ignore(true)
        .parents(false)
        .max_depth(Some(64));
    builder.filter_entry(move |entry| {
        entry.path().strip_prefix(&root).is_ok_and(|relative| {
            !policy::sensitive(entry.path())
                && !policy::excluded(relative)
                && (!entry.file_type().is_some_and(|kind| kind.is_dir())
                    || !entry.path().join("CACHEDIR.TAG").is_file())
        })
    });
    builder.build()
}

#[expect(
    clippy::too_many_arguments,
    reason = "one cancellable background job owns extraction and inference resources"
)]
fn index_file(
    path: &Path,
    revision: &FileRevision,
    config: &Config,
    connection: &mut rusqlite::Connection,
    root: &Path,
    shared: &Control,
    embedder: &mut Option<Embedder>,
    artifacts: &Path,
    cancellation: &crate::sandbox::Cancellation,
    cancelled: &impl Fn() -> bool,
) -> Result<bool, String> {
    let cursor = store::cursor(connection, path, revision)?;
    let extraction = if policy::image(path) {
        Extraction {
            segments: vec![ExtractedSegment {
                locator: ContentLocator::Image,
                text: String::new(),
                modality: "image".into(),
            }],
            next: None,
        }
    } else if let Some(video) = policy::media(path) {
        return index_media(
            path,
            revision,
            config,
            connection,
            root,
            shared,
            embedder,
            artifacts,
            cancellation,
            cancelled,
            video,
            cursor,
        );
    } else if policy::plain(path)
        || matches!(
            policy::extension(path).as_str(),
            "pdf" | "docx" | "odt" | "xlsx" | "xls" | "ods" | "pptx" | "odp"
        )
    {
        let output = crate::sandbox::parse(
            path,
            crate::sandbox::ParseOperation::SearchExtract,
            cursor as i32,
            crate::sandbox::MediaPreviewBackend::Software,
            cancellation,
        )?;
        validate_extraction(&output.data)?
    } else {
        return Err("unsupported-format".into());
    };
    let mut vectors = vec![None; extraction.segments.len()];
    if config.enabled && model::ready(&shared.data) {
        if embedder.is_none() {
            *embedder = Embedder::start(&shared.data, artifacts, false, cancelled).ok();
        }
        if let Some(worker) = embedder.as_mut() {
            for (batch_index, batch) in extraction.segments.chunks(8).enumerate() {
                if cancelled() {
                    return Err("Content indexing cancelled".into());
                }
                let (operation, inputs) = if batch.len() == 1
                    && batch[0].text.is_empty()
                    && let Some(page) = batch[0].locator.page()
                {
                    let output = crate::sandbox::parse(
                        path,
                        crate::sandbox::ParseOperation::PreviewPdf(
                            crate::sandbox::PdfRenderSize::new(640, 800),
                        ),
                        page as i32,
                        crate::sandbox::MediaPreviewBackend::Software,
                        cancellation,
                    )?;
                    let file = artifacts.join("pdf-page.png");
                    fs::write(&file, output.data).map_err(|error| error.to_string())?;
                    ("image", vec![file.to_string_lossy().into_owned()])
                } else if policy::image(path) {
                    let file = prepare_image(path, artifacts, cancellation)?;
                    ("image", vec![file.to_string_lossy().into_owned()])
                } else {
                    (
                        "text",
                        batch
                            .iter()
                            .map(|segment| {
                                format!(
                                    "title: {} | text: {}",
                                    path.file_name().unwrap_or_default().to_string_lossy(),
                                    segment.text.chars().take(4000).collect::<String>()
                                )
                            })
                            .collect(),
                    )
                };
                match worker.embed(operation, &inputs, cancelled) {
                    Ok(rows) => {
                        for (index, vector) in rows.into_iter().enumerate() {
                            vectors[batch_index * 8 + index] = vector;
                        }
                    }
                    Err(error) => {
                        *embedder = None;
                        tracing::debug!(reason=%error,"semantic indexing falls back to keywords");
                        break;
                    }
                }
            }
        }
    }
    if cancelled()
        || fs::metadata(path)
            .map(|metadata| FileRevision::from_metadata(&metadata) != *revision)
            .unwrap_or(true)
    {
        return Err("Content file changed during extraction".into());
    }
    store::append(
        connection,
        root,
        path,
        revision,
        &extraction.segments,
        &vectors,
        Some(model::FINGERPRINT),
        extraction.next,
        cursor > 0,
    )?;
    Ok(extraction.next.is_some())
}

fn validate_extraction(bytes: &[u8]) -> Result<Extraction, String> {
    if bytes.len() > 1024 * 1024 {
        return Err("Content extraction response exceeds its budget".into());
    }
    let value: Extraction = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if value.segments.len() > 1024
        || value
            .segments
            .iter()
            .any(|segment| segment.text.len() > 16000 || segment.modality.len() > 32)
    {
        return Err("Invalid content extraction response".into());
    }
    Ok(value)
}

fn prepare_image(
    path: &Path,
    artifacts: &Path,
    cancellation: &crate::sandbox::Cancellation,
) -> Result<PathBuf, String> {
    let output = crate::sandbox::parse(
        path,
        crate::sandbox::ParseOperation::PreviewImage,
        0,
        crate::sandbox::MediaPreviewBackend::Software,
        cancellation,
    )?;
    let destination = artifacts.join("image.png");
    #[cfg(windows)]
    if let Some((width, height)) = output.raster_dimensions {
        image::save_buffer_with_format(
            &destination,
            &output.data,
            width,
            height,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .map_err(|error| error.to_string())?;
        return Ok(destination);
    }
    if output.data.starts_with(b"STRGBA01") {
        let ((width, height), pixels) = crate::sandbox::pixels::decode(output.data)?;
        image::save_buffer_with_format(
            &destination,
            &pixels,
            width,
            height,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .map_err(|error| error.to_string())?;
    } else {
        fs::write(&destination, &output.data).map_err(|error| error.to_string())?;
    }
    Ok(destination)
}

#[expect(
    clippy::too_many_arguments,
    reason = "one background file job shares its existing resource and cancellation owners"
)]
fn index_media(
    path: &Path,
    revision: &FileRevision,
    config: &Config,
    connection: &mut rusqlite::Connection,
    root: &Path,
    shared: &Control,
    embedder: &mut Option<Embedder>,
    artifacts: &Path,
    cancellation: &crate::sandbox::Cancellation,
    cancelled: &impl Fn() -> bool,
    video: bool,
    cursor: u32,
) -> Result<bool, String> {
    if !config.enabled || !model::ready(&shared.data) {
        return Err("models-not-ready".into());
    }
    if embedder.is_none() {
        *embedder = Some(Embedder::start(&shared.data, artifacts, false, cancelled)?);
    }
    super::media::index(
        path,
        revision,
        connection,
        root,
        embedder.as_mut().ok_or("Embedding runtime unavailable")?,
        artifacts,
        cancellation,
        cancelled,
        video,
        cursor,
    )
}

fn on_battery() -> bool {
    #[cfg(windows)]
    {
        strata_win_shell::power::on_battery()
    }
    #[cfg(target_os = "linux")]
    {
        fs::read_dir("/sys/class/power_supply")
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| {
                fs::read_to_string(entry.path().join("type"))
                    .is_ok_and(|kind| kind.trim() == "Battery")
                    && fs::read_to_string(entry.path().join("status"))
                        .is_ok_and(|state| state.trim() == "Discharging")
            })
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        false
    }
}
