// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

mod remote;

use futures_channel::oneshot;
use gtk::{gio, glib, prelude::*};

use crate::{
    adapters::{
        local_operations::{ArchiveListingStatus, decode_archive_listing},
        read_file_for_location,
    },
    sandbox::{Cancellation, MediaPreviewBackend, ParseOperation, PdfRenderSize},
    services::{
        LoadHandle, MediaPreviewSize, ModelFormat, ModelRender, Preview, PreviewContent,
        PreviewEvent, PreviewProvider, PreviewRequest, SandboxedMedia, content_family_for_file,
        document_kind, has_plain_text_extension, is_gif_path, is_image_path, is_media_path,
        is_non_executable_extensionless_dotfile, layout_document, normalize_preview_text,
        parse_document,
    },
};

const MAX_PREVIEW_CACHE_ENTRIES: usize = 64;
const MAX_PREVIEW_CACHE_BYTES: usize = 128 * 1024 * 1024;
const MAX_CONCURRENT_HEAVY_PREVIEWS: usize = 1;

#[derive(Default)]
struct HeavyPreviewQueue {
    running: usize,
    next_id: u64,
    queued: VecDeque<(u64, oneshot::Sender<HeavyPreviewPermit>)>,
}

struct HeavyPreviewWaiter {
    id: u64,
    receive: Option<oneshot::Receiver<HeavyPreviewPermit>>,
}

impl HeavyPreviewWaiter {
    async fn acquire(mut self) -> Option<HeavyPreviewPermit> {
        self.receive.take()?.await.ok()
    }
}

impl Drop for HeavyPreviewWaiter {
    fn drop(&mut self) {
        HEAVY_PREVIEW_QUEUE.with(|queue| {
            queue.borrow_mut().queued.retain(|(id, _)| *id != self.id);
        });
    }
}

struct HeavyPreviewPermit;

impl Drop for HeavyPreviewPermit {
    fn drop(&mut self) {
        release_heavy_preview_permit();
    }
}

fn request_heavy_preview_permit() -> HeavyPreviewWaiter {
    let (send, receive) = oneshot::channel();
    let (id, start) = HEAVY_PREVIEW_QUEUE.with(|queue| {
        let mut queue = queue.borrow_mut();
        let id = queue.next_id;
        queue.next_id = queue.next_id.saturating_add(1);
        if queue.running < MAX_CONCURRENT_HEAVY_PREVIEWS {
            queue.running += 1;
            (id, Some(send))
        } else {
            queue.queued.push_back((id, send));
            (id, None)
        }
    });
    if let Some(send) = start
        && let Err(permit) = send.send(HeavyPreviewPermit)
    {
        drop(permit);
    }
    HeavyPreviewWaiter {
        id,
        receive: Some(receive),
    }
}

fn release_heavy_preview_permit() {
    let next = HEAVY_PREVIEW_QUEUE.with(|queue| {
        let mut queue = queue.borrow_mut();
        queue.running = queue.running.saturating_sub(1);
        let next = queue.queued.pop_front().map(|(_, send)| send);
        if next.is_some() {
            queue.running += 1;
        }
        next
    });
    if let Some(send) = next
        && let Err(permit) = send.send(HeavyPreviewPermit)
    {
        drop(permit);
    }
}

#[derive(Default)]
struct PreviewCache {
    entries: HashMap<PreviewCacheKey, PreviewContent>,
    recent: VecDeque<PreviewCacheKey>,
    byte_count: usize,
    last_used: Option<Instant>,
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct PreviewCacheKey {
    fast_raw: bool,
    raw_develop: bool,
    revision: Option<crate::services::smart_search::result::FileRevision>,
    path: PathBuf,
    modified: i64,
    pdf_page: Option<(i32, PdfRenderSize)>,
    model: Option<ModelRender>,
}

impl PreviewCache {
    fn clear_if_idle(&mut self, now: Instant) -> bool {
        if self
            .last_used
            .is_none_or(|used| now.saturating_duration_since(used) < Duration::from_secs(90))
        {
            return false;
        }
        let populated = !self.entries.is_empty();
        self.entries = HashMap::new();
        self.recent = VecDeque::new();
        self.byte_count = 0;
        self.last_used = None;
        populated
    }

    fn get(&mut self, key: &PreviewCacheKey) -> Option<PreviewContent> {
        self.clear_if_idle(Instant::now());
        let content = self.entries.get(key)?.clone();
        self.last_used = Some(Instant::now());
        self.recent.retain(|k| k != key);
        self.recent.push_back(key.clone());
        Some(content)
    }

    fn insert(&mut self, key: PreviewCacheKey, content: PreviewContent) {
        if matches!(content, PreviewContent::SandboxedMedia { .. }) {
            return;
        }
        self.last_used = Some(Instant::now());
        let bytes = preview_content_size(&content);
        self.recent.retain(|k| k != &key);
        if let Some(old) = self.entries.remove(&key) {
            self.byte_count = self.byte_count.saturating_sub(preview_content_size(&old));
        }
        self.byte_count = self.byte_count.saturating_add(bytes);
        self.recent.push_back(key.clone());
        self.entries.insert(key, content);
        while self.entries.len() > MAX_PREVIEW_CACHE_ENTRIES
            || self.byte_count > MAX_PREVIEW_CACHE_BYTES
        {
            let Some(oldest) = self.recent.pop_front() else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.byte_count = self
                    .byte_count
                    .saturating_sub(preview_content_size(&removed));
            }
        }
    }
}

fn preview_content_size(content: &PreviewContent) -> usize {
    match content {
        PreviewContent::Rasterized { png } | PreviewContent::Model { png, .. } => png.capacity(),
        PreviewContent::RawImage { pixels, .. } => pixels.capacity(),
        PreviewContent::Pdf {
            png, text_layer, ..
        } => {
            png.capacity()
                + text_layer.as_ref().map_or(0, |layer| {
                    layer.text.capacity() + layer.glyphs.capacity() * size_of::<[f32; 4]>()
                })
        }
        PreviewContent::SandboxedMedia { .. } => 0,
        PreviewContent::Text { content, .. } => content.capacity(),
        PreviewContent::Document {
            source,
            document,
            fallback_reason,
            warnings,
            ..
        } => {
            source.capacity()
                + document.as_ref().map_or(0, document_retained_bytes)
                + fallback_reason.as_ref().map_or(0, String::capacity)
                + warnings.capacity() * size_of::<String>()
                + warnings.iter().map(String::capacity).sum::<usize>()
        }
        PreviewContent::Rendered { document, warnings } => {
            document_retained_bytes(document)
                + warnings.capacity() * size_of::<String>()
                + warnings.iter().map(String::capacity).sum::<usize>()
        }
        PreviewContent::Archive { tree } => {
            let mut bytes = 0;
            let mut pending = vec![&tree.root];
            while let Some(directory) = pending.pop() {
                bytes += directory.name.capacity()
                    + directory.children.capacity() * size_of::<crate::services::ArchiveNode>();
                for child in &directory.children {
                    match child {
                        crate::services::ArchiveNode::Directory(directory) => {
                            pending.push(directory)
                        }
                        crate::services::ArchiveNode::File { name, .. } => bytes += name.capacity(),
                    }
                }
            }
            bytes
        }
        _ => 0,
    }
}

fn document_retained_bytes(document: &crate::services::DocumentLayout) -> usize {
    use crate::services::{
        DocumentMedia, DocumentSpan, DocumentSpanStyle, DocumentTableCellLayout, DocumentUnit,
        DocumentUnitKind,
    };
    fn spans_bytes(spans: &Vec<DocumentSpan>) -> usize {
        spans.capacity() * size_of::<DocumentSpan>()
            + spans
                .iter()
                .map(|span| match &span.style {
                    DocumentSpanStyle::Link(text) | DocumentSpanStyle::Math(text) => text.len(),
                    _ => 0,
                })
                .sum::<usize>()
    }
    let mut bytes = document.units.capacity() * size_of::<DocumentUnit>();
    for unit in &document.units {
        bytes += unit.text.capacity() + unit.copy_text.capacity() + spans_bytes(&unit.spans);
        match &unit.kind {
            DocumentUnitKind::Table { rows, .. } => {
                bytes += rows.capacity() * size_of::<Vec<DocumentTableCellLayout>>();
                for row in rows {
                    bytes += row.capacity() * size_of::<DocumentTableCellLayout>();
                    for cell in row {
                        bytes += cell.text.capacity() + spans_bytes(&cell.spans);
                    }
                }
            }
            DocumentUnitKind::Media { source, .. } => {
                bytes += match source {
                    DocumentMedia::Image(source)
                    | DocumentMedia::Mermaid(source)
                    | DocumentMedia::Math { source, .. } => source.capacity(),
                };
            }
            _ => {}
        }
    }
    bytes
}

thread_local! {
    static HEAVY_PREVIEW_QUEUE: RefCell<HeavyPreviewQueue> = RefCell::new(HeavyPreviewQueue::default());
    static PREVIEW_CACHE: RefCell<PreviewCache> = RefCell::new(PreviewCache {
        entries: HashMap::new(),
        recent: VecDeque::new(),
        byte_count: 0,
        last_used: None,
    });
    static CACHE_MAINTENANCE: RefCell<Weak<PreviewCacheMaintenance>> = const { RefCell::new(Weak::new()) };
}

struct PreviewCacheMaintenance {
    timer: Option<glib::SourceId>,
}

impl PreviewCacheMaintenance {
    fn shared() -> Rc<Self> {
        CACHE_MAINTENANCE.with_borrow_mut(|current| {
            if let Some(maintenance) = current.upgrade() {
                return maintenance;
            }
            // No thread-bound captures: another context owner may dispatch
            // during teardown. Each dispatch only accesses its own TLS cache.
            let timer = glib::timeout_add_seconds(30, || {
                PREVIEW_CACHE.with_borrow_mut(|cache| {
                    cache.clear_if_idle(Instant::now());
                });
                glib::ControlFlow::Continue
            });
            let maintenance = Rc::new(Self { timer: Some(timer) });
            *current = Rc::downgrade(&maintenance);
            maintenance
        })
    }
}

impl Drop for PreviewCacheMaintenance {
    fn drop(&mut self) {
        if let Some(timer) = self.timer.take() {
            timer.remove();
        }
        let _ = PREVIEW_CACHE.try_with(|cache| {
            *cache.borrow_mut() = PreviewCache::default();
        });
    }
}

pub struct LocalPreviewProvider {
    media_preview_backend: Rc<dyn Fn() -> MediaPreviewBackend>,
    _cache_maintenance: Rc<PreviewCacheMaintenance>,
}

impl LocalPreviewProvider {
    pub(crate) fn new(media_preview_backend: Rc<dyn Fn() -> MediaPreviewBackend>) -> Self {
        Self {
            media_preview_backend,
            _cache_maintenance: PreviewCacheMaintenance::shared(),
        }
    }
}

impl PreviewProvider for LocalPreviewProvider {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        use futures_lite::StreamExt;
        if !crate::services::is_model(&request.entry.native_name) {
            return self.load_with_renderer(request, emit, crate::sandbox::parse);
        }
        let (send, mut receive) = futures_channel::mpsc::channel(8);
        let send = RefCell::new(send);
        let request_id = request.id;
        let progress_emit = emit.clone();
        let progress_task = glib::MainContext::default().spawn_local(async move {
            while let Some(stage) = receive.next().await {
                progress_emit(PreviewEvent::Progress { request_id, stage });
            }
        });
        let load = self.load_with_renderer(
            request,
            emit,
            move |path, operation, value, backend, cancellation| {
                crate::sandbox::parse_with_progress(
                    path,
                    operation,
                    value,
                    backend,
                    cancellation,
                    &|stage| {
                        let _ = send.borrow_mut().try_send(stage);
                    },
                )
            },
        );
        LoadHandle::new(move || {
            drop(load);
            progress_task.abort();
        })
    }
}

impl LocalPreviewProvider {
    fn load_with_renderer(
        &self,
        mut request: PreviewRequest,
        emit: Rc<dyn Fn(PreviewEvent)>,
        render: impl FnOnce(
            &Path,
            ParseOperation,
            i32,
            MediaPreviewBackend,
            &Cancellation,
        ) -> Result<crate::sandbox::ParseOutput, String>
        + Send
        + 'static,
    ) -> LoadHandle {
        let media_preview_backend = (self.media_preview_backend)();
        let request_id = request.id;
        let entry = request.entry.clone();
        let cancellation = Cancellation::default();
        let cancellation_for_task = cancellation.clone();
        let abort_safe = Rc::new(Cell::new(true));
        let abort_safe_for_task = abort_safe.clone();
        let task = glib::MainContext::default().spawn_local(async move {
            if let Some(hit)=request.search_hit.clone(){
                let revision=hit.revision.clone();let path=hit.path.clone();
                let valid=gio::spawn_blocking(move||std::fs::metadata(path).is_ok_and(|metadata|crate::services::smart_search::result::FileRevision::from_metadata(&metadata)==revision)).await;
                if !matches!(valid,Ok(true)) {emit(PreviewEvent::Failed{request_id,entry,message:"The file changed; search again to locate this match".into()});return;}
                if let Some(page)=hit.locator.page(){request.pdf_page=page.min(i32::MAX as u32) as i32;}
            }
            let (guessed_type, uncertain) =
                crate::services::guess_content_type(Path::new(&entry.native_name));
            let mut content_type = guessed_type.to_string();
            let mut content = content_family_for_file(&entry.native_name, &content_type);
            if crate::services::is_model(&entry.native_name) {
                content = PreviewContent::Unsupported;
            }

            if !crate::services::is_model(&entry.native_name)
                && matches!(content, PreviewContent::Unsupported)
            {
                if gio::content_type_is_a(&content_type, "text/plain") {
                    content = PreviewContent::Text {
                        content: String::new(),
                        truncated: false,
                    };
                } else if has_plain_text_extension(&entry.native_name) {
                    content = PreviewContent::Text {
                        content: String::new(),
                        truncated: false,
                    };
                    content_type = "text/plain".to_owned();
                } else if is_gif_path(Path::new(&entry.native_name)) {
                    // Keep GIF animation when Windows has no MIME database.
                    // The media decoder bounds and loops the animation in the sandbox.
                    content = PreviewContent::Media;
                    content_type = "image/gif".to_owned();
                } else if is_image_path(Path::new(&entry.native_name)) {
                    // Windows GIO can report application/octet-stream for
                    // valid images when no desktop MIME database is present.
                    // Extension classification is only a fallback; the
                    // bounded decoder still validates the actual bytes.
                    content = PreviewContent::Image;
                    content_type = "image/*".to_owned();
                } else if is_media_path(Path::new(&entry.native_name)) {
                    content = PreviewContent::Media;
                    content_type = if crate::services::is_audio_path(Path::new(&entry.native_name)) {
                        "audio/*"
                    } else {
                        "video/*"
                    }.to_owned();
                }
            }

            if !crate::services::is_model(&entry.native_name) && matches!(content, PreviewContent::Unsupported)
                && (uncertain || entry.native_name.is_empty())
            {
                let file = read_file_for_location(&entry.location);
                let info = match file
                    .query_info_future(
                        "standard::content-type,unix::mode",
                        gio::FileQueryInfoFlags::NONE,
                        glib::Priority::DEFAULT,
                    )
                    .await
                {
                    Ok(info) => info,
                    Err(error) => {
                        emit(PreviewEvent::Failed {
                            request_id,
                            entry,
                            message: crate::services::gio_error_message(&error),
                        });
                        return;
                    }
                };
                let queried_type = info
                    .content_type()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "application/octet-stream".to_owned());
                let unix_mode = info
                    .has_attribute(gio::FILE_ATTRIBUTE_UNIX_MODE)
                    .then(|| info.attribute_uint32(gio::FILE_ATTRIBUTE_UNIX_MODE));
                let mut resolved = content_family_for_file(&entry.native_name, &queried_type);
                if matches!(resolved, PreviewContent::Unsupported)
                    && (gio::content_type_is_a(&queried_type, "text/plain")
                        || has_plain_text_extension(&entry.native_name)
                        || is_non_executable_extensionless_dotfile(&entry.native_name, unix_mode))
                {
                    resolved = PreviewContent::Text {
                        content: String::new(),
                        truncated: false,
                    };
                }
                content = resolved;
                content_type = queried_type;
            }

            if matches!(content, PreviewContent::Pdf { .. }) {
                content_type = "application/pdf".to_owned();
            }
            if let Some(format) = crate::services::archive_preview_format(&entry.native_name) {
                let archive_path = match entry.location.native_path() {
                    Some(path) => path.to_path_buf(),
                    None => {
                        emit(PreviewEvent::Failed {
                            request_id,
                            entry,
                            message: "Copy this archive locally before previewing it".into(),
                        });
                        return;
                    }
                };
                let cancellation = cancellation_for_task.clone();
                let password = request.archive_password.clone();
                let listed = gio::spawn_blocking(move || {
                    let output = render(
                        &archive_path,
                        ParseOperation::ArchiveList { format, password },
                        0,
                        MediaPreviewBackend::Software,
                        &cancellation,
                    )?;
                    decode_archive_listing(&output.data).map(|listing| {
                        let tree = crate::services::archive_preview_tree(listing.entries);
                        (listing.status, tree)
                    })
                })
                .await;
                if cancellation_for_task.is_cancelled() {
                    return;
                }
                match listed {
                    Ok(Ok((status, tree))) => match status {
                        ArchiveListingStatus::Open => {
                            emit(PreviewEvent::Ready(Preview {
                                request_id,
                                entry,
                                content_type,
                                content: PreviewContent::Archive { tree },
                            }));
                            return;
                        }
                        ArchiveListingStatus::NeedsPassword => {
                            emit(PreviewEvent::NeedsPassword { request_id, entry });
                            return;
                        }
                        ArchiveListingStatus::WrongPassword => {
                            emit(PreviewEvent::Failed {
                                request_id,
                                entry,
                                message: crate::services::INCORRECT_ARCHIVE_PASSWORD.to_owned(),
                            });
                            return;
                        }
                        ArchiveListingStatus::Unsupported => {
                            emit(PreviewEvent::Failed {
                                request_id,
                                entry,
                                message:
                                    crate::adapters::ARCHIVE_UNSUPPORTED_MESSAGE.to_owned(),
                            });
                            return;
                        }
                    },
                    Ok(Err(message)) => {
                        emit(PreviewEvent::Failed {
                            request_id,
                            entry,
                            message,
                        });
                        return;
                    }
                    Err(_) => return,
                }
            }

            let sandboxed = if crate::services::table::is_workbook(&content_type, &entry.native_name)
            {
                Some(ParseOperation::PreviewWorkbook)
            } else if crate::services::docx::is_document(&content_type, &entry.native_name) {
                Some(ParseOperation::PreviewDocument)
            } else {
                None
            };
            if let Some(operation) = sandboxed {
                let Some(path) = entry.location.native_path().map(ToOwned::to_owned) else {
                    emit(PreviewEvent::Failed { request_id, entry, message: "Copy this file locally before previewing it".into() });
                    return;
                };
                // Share the full-document parser slot with PDF rendering. Keep it
                // until the cancelled helper exits, not merely until the UI closes.
                let Some(permit) = request_heavy_preview_permit().acquire().await else {
                    return;
                };
                if cancellation_for_task.is_cancelled() {
                    return;
                }
                abort_safe_for_task.set(false);
                let cancellation = cancellation_for_task.clone();
                let result = gio::spawn_blocking(move || {
                    let is_workbook = matches!(operation, ParseOperation::PreviewWorkbook);
                    let output = render(&path, operation, 0, media_preview_backend, &cancellation)?;
                    let parsed = if is_workbook {
                        crate::services::table::TableData::from_json(&output.data)?.into_document()
                    } else {
                        crate::services::docx::RichTextData::from_json(&output.data)?.into_document(&cancellation)?
                    };
                    layout_document(parsed.document, &cancellation).map(|document| PreviewContent::Rendered { document, warnings: parsed.warnings })
                }).await;
                abort_safe_for_task.set(true);
                drop(permit);
                if cancellation_for_task.is_cancelled() { return; }
                match result {
                    Ok(Ok(content)) => emit(PreviewEvent::Ready(Preview { request_id, entry, content_type, content })),
                    Ok(Err(message)) => emit(PreviewEvent::Failed { request_id, entry, message }),
                    Err(_) => emit(PreviewEvent::Failed { request_id, entry, message: "The preview worker stopped unexpectedly.".into() }),
                }
                return;
            }

            let document_kind = document_kind(
                &content_type,
                &entry.native_name,
                entry.location.native_path().is_some(),
            );
            if document_kind.is_some() {
                content = PreviewContent::Document {
                    source: String::new(),
                    document: None,
                    fallback_reason: None,
                    warnings: Vec::new(),
                    truncated: false,
                };
            }

            if matches!(content, PreviewContent::Media) {
                let staged = if entry.location.native_path().is_none() {
                    if !crate::services::supports_remote_video(&entry.native_name) {
                        emit(PreviewEvent::Failed {
                            request_id,
                            entry,
                            message: "Copy this media format locally before previewing it".into(),
                        });
                        return;
                    }
                    match remote::stage_video(&entry).await {
                        Ok(staged) => Some(staged),
                        Err(message) => {
                            if !cancellation_for_task.is_cancelled() {
                                emit(PreviewEvent::Failed {
                                    request_id,
                                    entry,
                                    message,
                                });
                            }
                            return;
                        }
                    }
                } else {
                    None
                };
                let path = staged
                    .as_ref()
                    .map(|input| input.path())
                    .or_else(|| entry.location.native_path())
                    .expect("native or staged media")
                    .to_path_buf();
                let mut media = SandboxedMedia {
                    path,
                    size: request.media_size,
                    backend: media_preview_backend,
                    input_owner: None,
                    audio_only: content_type.starts_with("audio/")
                        || crate::services::is_audio_path(Path::new(&entry.native_name)),
                };
                if let Some(staged) = staged {
                    media = media.retain_input(staged);
                }
                if cancellation_for_task.is_cancelled() {
                    return;
                }
                emit(PreviewEvent::Ready(Preview {
                    request_id,
                    entry,
                    content_type,
                    content: PreviewContent::SandboxedMedia { media },
                }));
                return;
            }

            let model = ModelFormat::for_name(&entry.native_name).map(|format| ModelRender {
                format,
                size: MediaPreviewSize::new(request.media_size.width.min(800), request.media_size.height.min(800)),
                palette: request.model_palette,
            });
            let operation = if let Some(render) = model {
                Some(ParseOperation::PreviewModel(render))
            } else if let Some(format) = crate::sandbox::CoverFormat::for_name(&entry.native_name) {
                Some(ParseOperation::PreviewCover(format))
            } else { match content {
                PreviewContent::Pdf { .. } => Some(ParseOperation::PreviewPdf(pdf_render_size(
                    request.media_size,
                ))),
                PreviewContent::Image => Some(if request.raw_develop {ParseOperation::PreviewRawDevelop} else if crate::sandbox::raw_metadata::is_raw(std::path::Path::new(&entry.native_name)) && !crate::sandbox::pixels::fast_raw() { ParseOperation::PreviewImageLegacy } else { ParseOperation::PreviewImage }),
                PreviewContent::Media => None,
                PreviewContent::Text { .. }
                | PreviewContent::Model { .. }
                | PreviewContent::Document { .. }
                | PreviewContent::Rendered { .. }
                | PreviewContent::Rasterized { .. }
                | PreviewContent::RawImage { .. }
                | PreviewContent::SandboxedMedia { .. }
                | PreviewContent::Archive { .. }
                | PreviewContent::Unsupported => None,
            }};
            if let Some(operation) = &operation {
                let staged = if entry.location.native_path().is_none() {
                    if !matches!(operation, ParseOperation::PreviewImage) {
                        emit(PreviewEvent::Failed {
                            request_id,
                            entry,
                            message: if matches!(operation, ParseOperation::PreviewModel(_)) {
                                "Remote model previews are not supported; copy the file locally first"
                            } else if matches!(operation, ParseOperation::PreviewCover(_)) {
                                "Remote cover previews are not supported; copy the file locally first"
                            } else {
                                "Remote PDF previews are not supported; copy the file locally first"
                            }.into(),
                        });
                        return;
                    }
                    match remote::stage(&entry).await {
                        Ok(staged) => Some(staged),
                        Err(message) => {
                            if !cancellation_for_task.is_cancelled() {
                                emit(PreviewEvent::Failed {
                                    request_id,
                                    entry,
                                    message,
                                });
                            }
                            return;
                        }
                    }
                } else {
                    None
                };
                let path = staged
                    .as_ref()
                    .map(|file| file.path())
                    .or_else(|| entry.location.native_path())
                    .expect("native or staged preview input")
                    .to_path_buf();
                // Remote originals are not written to persistent thumbnail caches.
                let modified = match entry.modified_unix_seconds {
                    _ if staged.is_some() => None,
                    crate::model::MetadataValue::Known(m) => Some(m),
                    crate::model::MetadataValue::Unknown
                    | crate::model::MetadataValue::Unavailable => None,
                };
                let pdf_page = match operation {
                    ParseOperation::PreviewPdf(size) => Some((request.pdf_page, *size)),
                    _ => None,
                };
                let cache_key = modified.map(|modified| PreviewCacheKey {
                    fast_raw: crate::sandbox::pixels::fast_raw(),
                    raw_develop: request.raw_develop,
                    revision: request.search_hit.as_ref().map(|hit|hit.revision.clone()),
                    path: path.clone(),
                    modified,
                    pdf_page,
                    model,
                });
                if let Some(cached) = cache_key
                    .as_ref()
                    .and_then(|key| PREVIEW_CACHE.with(|cache| cache.borrow_mut().get(key)))
                {
                    emit(PreviewEvent::Ready(Preview {
                        request_id,
                        entry,
                        content_type,
                        content: cached,
                    }));
                    return;
                }

                if cancellation_for_task.is_cancelled() {
                    return;
                }

                let heavy_permit = if matches!(operation, ParseOperation::PreviewPdf(_) | ParseOperation::PreviewModel(_) | ParseOperation::PreviewCover(_) | ParseOperation::PreviewRawDevelop | ParseOperation::PreviewImageLegacy)
                    || (matches!(operation,ParseOperation::PreviewImage) && crate::sandbox::raw_metadata::is_raw(Path::new(&entry.native_name))) {
                    let Some(permit) = request_heavy_preview_permit().acquire().await else {
                        return;
                    };
                    if cancellation_for_task.is_cancelled() {
                        return;
                    }
                    Some(permit)
                } else {
                    None
                };
                abort_safe_for_task.set(heavy_permit.is_none());
                let value = request.pdf_page;
                let cancellation = cancellation_for_task.clone();
                let spawn_path = path.clone();
                let mut thumbnail_to_store = None;
                let for_render = operation.clone();
                let expected_revision=request.search_hit.as_ref().map(|hit|hit.revision.clone());
                let render = gio::spawn_blocking(move || {
                    let _staged = staged;
                    let output = render(
                        &spawn_path,
                        for_render,
                        value,
                        media_preview_backend,
                        &cancellation,
                    )?;
                    if let Some(expected)=expected_revision && !std::fs::metadata(&spawn_path).is_ok_and(|metadata|crate::services::smart_search::result::FileRevision::from_metadata(&metadata)==expected){return Err("The file changed; search again to locate this match".into());}
                    Ok::<_, String>(output)
                })
                .await;
                abort_safe_for_task.set(true);
                if cancellation_for_task.is_cancelled() {
                    return;
                }
                content = match render {
                    Ok(Ok(output)) if model.is_some() => PreviewContent::Model {
                        png: output.data,
                    },
                    Ok(Ok(output)) if matches!(operation, ParseOperation::PreviewPdf(_)) => {
                        if let Some(mtime) = modified
                            && request.pdf_page == 0
                        {
                            thumbnail_to_store = Some((path.clone(), mtime, output.data.clone()));
                        }
                        PreviewContent::Pdf {
                            png: output.data,
                            page: output.page,
                            pages: output.pages,
                            text_layer: output.text_layer.map(std::sync::Arc::new),
                        }
                    }
                    #[cfg(windows)]
                    Ok(Ok(output)) if matches!(operation, ParseOperation::PreviewImage | ParseOperation::PreviewImageLegacy | ParseOperation::PreviewRawDevelop)
                        && output.raster_dimensions.is_some() => {
                        let (width, height) = output.raster_dimensions.expect("validated image dimensions");
                        PreviewContent::RawImage { width, height, pixels: output.data }
                    }
                    Ok(Ok(output)) if output.data.starts_with(b"STRGBA01") => {
                        match crate::sandbox::pixels::decode(output.data) {
                            Ok(((width,height), pixels)) => PreviewContent::RawImage { width, height, pixels },
                            Err(message) => { emit(PreviewEvent::Failed { request_id, entry, message }); return; }
                        }
                    }
                    Ok(Ok(output)) => {
                        if let Some(mtime) = modified.filter(|_| !matches!(operation, ParseOperation::PreviewCover(_))) {
                            thumbnail_to_store = Some((path.clone(), mtime, output.data.clone()));
                        }
                        PreviewContent::Rasterized { png: output.data }
                    }
                    Ok(Err(message)) => {
                        emit(PreviewEvent::Failed {
                            request_id,
                            entry,
                            message,
                        });
                        return;
                    }
                    Err(_) => {
                        if cancellation_for_task.is_cancelled() {
                            return;
                        }
                        emit(PreviewEvent::Failed {
                            request_id,
                            entry,
                            message: "The preview worker stopped unexpectedly.".to_owned(),
                        });
                        return;
                    }
                };
                drop(heavy_permit);
                if let Some(cache_key) = cache_key {
                    PREVIEW_CACHE.with(|cache| {
                        cache.borrow_mut().insert(cache_key, content.clone());
                    });
                }
                emit(PreviewEvent::Ready(Preview {
                    request_id,
                    entry,
                    content_type,
                    content,
                }));
                if let Some((path, mtime, png)) = thumbnail_to_store {
                    let _ = gio::spawn_blocking(move || {
                        crate::ui::thumbnail_cache::store(&path, mtime, &png);
                    })
                    .await;
                }
                return;
            } else if matches!(
                content,
                PreviewContent::Text { .. } | PreviewContent::Document { .. }
            ) {
                let file = read_file_for_location(&entry.location);
                let native_path = entry.location.native_path().map(ToOwned::to_owned);
                content =
                    match read_text(&file, native_path.as_deref(), request.text_byte_limit,request.search_hit.as_ref().map(|hit|hit.revision.clone())).await {
                        Ok((source, truncated)) => {
                            if let Some(kind) = document_kind {
                                if truncated {
                                    PreviewContent::Document {
                                        source,
                                        document: None,
                                        fallback_reason: Some(
                                            "Rendered view is unavailable because the document exceeds the 1 MB preview limit."
                                                .to_owned(),
                                        ),
                                        warnings: Vec::new(),
                                        truncated,
                                    }
                                } else if request.render_document {
                                    let cancellation = cancellation_for_task.clone();
                                    let parsed = gio::spawn_blocking(move || {
                                        let parsed = parse_document(kind, &source, &cancellation)
                                            .and_then(|parsed| {
                                                layout_document(parsed.document, &cancellation)
                                                    .map(|document| (document, parsed.warnings))
                                            });
                                        (source, parsed)
                                    })
                                    .await;
                                    let (source, parsed) = match parsed {
                                        Ok(parsed) => parsed,
                                        Err(_) => {
                                            if cancellation_for_task.is_cancelled() {
                                                return;
                                            }
                                            emit(PreviewEvent::Failed {
                                                request_id,
                                                entry,
                                                message: "The preview worker stopped unexpectedly."
                                                    .to_owned(),
                                            });
                                            return;
                                        }
                                    };
                                    if cancellation_for_task.is_cancelled() {
                                        return;
                                    }
                                    match parsed {
                                        Ok((document, warnings)) => PreviewContent::Document {
                                            source,
                                            document: Some(document),
                                            fallback_reason: None,
                                            warnings,
                                            truncated,
                                        },
                                        Err(reason) => PreviewContent::Document {
                                            source,
                                            document: None,
                                            fallback_reason: Some(reason),
                                            warnings: Vec::new(),
                                            truncated,
                                        },
                                    }
                                } else {
                                    PreviewContent::Document {
                                        source,
                                        document: None,
                                        fallback_reason: None,
                                        warnings: Vec::new(),
                                        truncated,
                                    }
                                }
                            } else {
                                PreviewContent::Text {
                                    content: source,
                                    truncated,
                                }
                            }
                        }
                        Err(error) => {
                            emit(PreviewEvent::Failed {
                                request_id,
                                entry,
                                message: crate::services::gio_error_message(&error),
                            });
                            return;
                        }
                    };
            }

            if cancellation_for_task.is_cancelled() {
                return;
            }
            emit(PreviewEvent::Ready(Preview {
                request_id,
                entry,
                content_type,
                content,
            }));
        });

        LoadHandle::new(move || {
            cancellation.cancel();
            if abort_safe.get() {
                task.abort();
            }
        })
    }
}

fn pdf_render_size(viewport: MediaPreviewSize) -> PdfRenderSize {
    PdfRenderSize::for_viewport_width(viewport.width)
}

async fn read_text(
    file: &gio::File,
    native_path: Option<&Path>,
    byte_limit: usize,
    revision: Option<crate::services::smart_search::result::FileRevision>,
) -> Result<(String, bool), glib::Error> {
    let byte_limit = byte_limit.min(16 * 1024 * 1024);
    if let Some(path) = native_path {
        let path = path.to_path_buf();
        let result = gio::spawn_blocking(move || {
            use std::io::Read;
            let io_failure = |error: std::io::Error| {
                glib::Error::new(
                    gio::IOErrorEnum::Failed,
                    &crate::services::io_error_message(&error),
                )
            };
            let file = std::fs::File::open(&path).map_err(io_failure)?;
            let mut bytes = Vec::new();
            file.take(byte_limit as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(io_failure)?;
            if let Some(expected) = revision
                && !std::fs::metadata(&path).is_ok_and(|metadata| {
                    crate::services::smart_search::result::FileRevision::from_metadata(&metadata)
                        == expected
                })
            {
                return Err(glib::Error::new(
                    gio::IOErrorEnum::Failed,
                    "The file changed; search again to locate this match",
                ));
            }
            let truncated = bytes.len() > byte_limit;
            let sample = &bytes[..bytes.len().min(byte_limit)];
            Ok((decode_text_sample(sample), truncated))
        })
        .await;
        match result {
            Ok(ok) => return ok,
            Err(_) => {
                return Err(glib::Error::new(
                    gio::IOErrorEnum::Cancelled,
                    "Read cancelled",
                ));
            }
        }
    }
    let stream = file.read_future(glib::Priority::DEFAULT).await?;
    let bytes = stream
        .read_bytes_future(byte_limit.saturating_add(1), glib::Priority::DEFAULT)
        .await?;
    let bytes = bytes.as_ref();
    let truncated = bytes.len() > byte_limit;
    let sample = &bytes[..bytes.len().min(byte_limit)];
    Ok((decode_text_sample(sample), truncated))
}

pub(crate) fn clear_preview_cache() {
    let discarded = PREVIEW_CACHE.with_borrow_mut(std::mem::take);
    drop(discarded);
}

fn decode_text_sample(sample: &[u8]) -> String {
    let decoded = if sample.starts_with(&[0xff, 0xfe]) || sample.starts_with(&[0xfe, 0xff]) {
        let little = sample[0] == 0xff;
        String::from_utf16_lossy(
            &sample[2..]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| {
                    if little {
                        u16::from_le_bytes([pair[0], pair[1]])
                    } else {
                        u16::from_be_bytes([pair[0], pair[1]])
                    }
                })
                .collect::<Vec<_>>(),
        )
    } else {
        String::from_utf8_lossy(sample)
            .trim_start_matches('\u{feff}')
            .to_owned()
    };
    normalize_preview_text(&decoded).into_owned()
}

#[cfg(all(test, not(windows)))]
mod tests;
