// SPDX-License-Identifier: MIT

use super::{
    Config, Control, clock, model, policy,
    result::{ContentLocator, ExtractedSegment, Extraction, FileRevision},
    runtime::Embedder,
    store,
};
use notify::{RecursiveMode, Watcher};
use std::{
    collections::{HashMap, HashSet, VecDeque},
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
            shared.folder_counts.store(Arc::new(HashMap::new()));
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
                .map(|root| {
                    policy::normalize_path(&fs::canonicalize(root).unwrap_or_else(|_| root.clone()))
                })
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
            refresh_counts(&shared, &config, &regular, &code);
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
                refresh_counts(&shared, &config, &regular, &code);
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
                    refresh_counts(&shared, &config, &regular, &code);
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
                refresh_counts(&shared, &config, &regular, &code);
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
                if error != "unsupported-format" && error != "models-not-ready" {
                    embedder = None;
                }
                let _ = store::mark(connection, &root, &path, &file_revision, &error);
                refresh_counts(&shared, &config, &regular, &code);
                tracing::debug!(reason=%error,"content indexing skipped one file");
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn refresh_counts(
    shared: &Control,
    config: &Config,
    regular: &Option<rusqlite::Connection>,
    code: &Option<rusqlite::Connection>,
) {
    let mut indexed = 0usize;
    let mut skipped = 0usize;
    let mut by_root: HashMap<String, usize> = HashMap::new();
    let connections = if config.include_code {
        [regular.as_ref(), code.as_ref()]
    } else {
        [regular.as_ref(), None]
    };
    for connection in connections.into_iter().flatten() {
        if let Ok(mut statement) =
            connection.prepare("SELECT root, state, count(*) FROM files GROUP BY root, state")
            && let Ok(rows) = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?.max(0) as usize,
                ))
            })
        {
            for (root, state, count) in rows.flatten() {
                match state.as_str() {
                    "indexed" => {
                        indexed += count;
                        let key = policy::normalize_path(Path::new(&root))
                            .to_string_lossy()
                            .into_owned();
                        *by_root.entry(key).or_insert(0) += count;
                    }
                    "skipped" => skipped += count,
                    _ => {}
                }
            }
        }
    }
    let mut folder_counts = HashMap::new();
    for root in &config.roots {
        let normalized =
            policy::normalize_path(&fs::canonicalize(root).unwrap_or_else(|_| root.clone()));
        let key = normalized.to_string_lossy();
        let count = by_root.get(key.as_ref()).copied().unwrap_or(0);
        folder_counts.insert(root.clone(), count);
        folder_counts.insert(normalized, count);
    }
    shared.indexed.store(indexed, Ordering::Relaxed);
    shared.skipped.store(skipped, Ordering::Relaxed);
    shared.folder_counts.store(Arc::new(folder_counts));
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
    let mut prepared_image = None;
    let extraction = if policy::image(path) {
        let (file, ocr_text) = prepare_image(path, artifacts, cancellation)?;
        prepared_image = Some(file);
        Extraction {
            segments: vec![ExtractedSegment {
                locator: ContentLocator::Image,
                text: ocr_text,
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
                    let file = match &prepared_image {
                        Some(file) => file.clone(),
                        None => prepare_image(path, artifacts, cancellation)?.0,
                    };
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
) -> Result<(PathBuf, String), String> {
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
        let ocr_text = strata_win_shell::ocr::recognize_rgba(width, height, &output.data)
            .unwrap_or_default()
            .chars()
            .take(4000)
            .collect();
        image::save_buffer_with_format(
            &destination,
            &output.data,
            width,
            height,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .map_err(|error| error.to_string())?;
        return Ok((destination, ocr_text));
    }
    if output.data.starts_with(b"STRGBA01") {
        let ((width, height), pixels) = crate::sandbox::pixels::decode(output.data)?;
        #[cfg(windows)]
        let ocr_text = strata_win_shell::ocr::recognize_rgba(width, height, &pixels)
            .unwrap_or_default()
            .chars()
            .take(4000)
            .collect();
        #[cfg(not(windows))]
        let ocr_text = String::new();
        image::save_buffer_with_format(
            &destination,
            &pixels,
            width,
            height,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .map_err(|error| error.to_string())?;
        return Ok((destination, ocr_text));
    }
    let ocr_text = recognize_png_text(&output.data);
    fs::write(&destination, &output.data).map_err(|error| error.to_string())?;
    Ok((destination, ocr_text))
}

pub(super) fn recognize_png_text(png_bytes: &[u8]) -> String {
    #[cfg(windows)]
    {
        if let Ok(image) = image::load_from_memory(png_bytes) {
            let rgba = image.to_rgba8();
            if let Some(text) =
                strata_win_shell::ocr::recognize_rgba(rgba.width(), rgba.height(), rgba.as_raw())
            {
                return text.chars().take(4000).collect();
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = png_bytes;
    }
    String::new()
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
