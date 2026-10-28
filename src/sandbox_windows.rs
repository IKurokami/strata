// SPDX-License-Identifier: MIT
//! Native Windows sandbox boundary using Win32 Job Objects.
//!
//! Provides strict isolation for untrusted user files:
//! - Child processes are pinned to an anonymous Win32 Job Object.
//! - Single process limit (`JOB_OBJECT_LIMIT_ACTIVE_PROCESS = 1`) prevents subprocesses.
//! - Hard memory quota (1 GiB) prevents unbounded memory exhaustion.
//! - `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` ensures children die if parent dies.
//! - Full UI restrictions (no clipboard, desktop, or window handle access).
#[path = "sandbox/pixels.rs"]
pub(crate) mod pixels;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use crate::services::{
    ArchiveFormat, MediaPreviewSize, ModelFormat, ModelRender, SecretString,
    model_preview::MAX_MODEL_INPUT_BYTES,
};

#[path = "sandbox/browser_windows.rs"]
pub(crate) mod browser;
#[path = "sandbox/media_windows.rs"]
pub(crate) mod media;
#[path = "sandbox/metadata.rs"]
pub(crate) mod metadata;
#[path = "sandbox/raw_metadata.rs"]
pub(crate) mod raw_metadata;

const WALL_TIME_LIMIT: Duration = Duration::from_secs(12);
pub(crate) const FILE_SIZE_LIMIT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_RASTER_INPUT_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const MAX_OUTPUT_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const MAX_TEXT_LAYER_BYTES: u64 = 8 * 1024 * 1024;
static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum MediaPreviewBackend {
    Automatic,
    VaApi,
    Vulkan,
    Software,
}

impl MediaPreviewBackend {
    pub(crate) fn argument(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::VaApi => "vaapi",
            Self::Vulkan => "vulkan",
            Self::Software => "software",
        }
    }

    pub(crate) fn from_argument(value: &str) -> Option<Self> {
        match value {
            "automatic" => Some(Self::Automatic),
            "vaapi" => Some(Self::VaApi),
            "vulkan" => Some(Self::Vulkan),
            "software" => Some(Self::Software),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PdfRenderSize {
    pub(crate) width: i32,
    pub(crate) height: i32,
}

impl PdfRenderSize {
    const MAX_WIDTH: i32 = 1_400;
    const MAX_HEIGHT: i32 = 1_800;
    const MAX_PIXELS: u64 = 2_500_000;

    pub(crate) fn new(width: i32, height: i32) -> Self {
        Self {
            width: width.clamp(16, Self::MAX_WIDTH),
            height: height.clamp(16, Self::MAX_HEIGHT),
        }
    }

    pub(crate) fn for_viewport_width(width: i32) -> Self {
        Self::new(width, Self::MAX_HEIGHT)
    }

    pub(crate) fn image_limits(self) -> (u32, u32, u64) {
        let size = Self::new(self.width, self.height);
        let width = size.width as u32;
        let height = size.height as u32;
        (
            width,
            height,
            (u64::from(width) * u64::from(height)).min(Self::MAX_PIXELS),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoverFormat {
    Cbz,
    Cbr,
    Epub,
}

impl CoverFormat {
    pub(crate) fn for_name(name: &std::ffi::OsStr) -> Option<Self> {
        let extension = Path::new(name).extension()?.to_str()?;
        Self::from_argument(&extension.to_ascii_lowercase())
    }

    pub(crate) fn argument(self) -> &'static str {
        match self {
            Self::Cbz => "cbz",
            Self::Cbr => "cbr",
            Self::Epub => "epub",
        }
    }

    pub(crate) fn from_argument(value: &str) -> Option<Self> {
        match value {
            "cbz" => Some(Self::Cbz),
            "cbr" => Some(Self::Cbr),
            "epub" => Some(Self::Epub),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ParseOperation {
    ThumbnailImage,
    ThumbnailRaw,
    ThumbnailRawLegacy,
    ThumbnailPdf,
    ThumbnailVideo,
    ThumbnailAppImage,
    ThumbnailModel(ModelFormat),
    ThumbnailCover(CoverFormat),
    PreviewCover(CoverFormat),
    PreviewImage,
    PreviewImageLegacy,
    PreviewRawDevelop,
    InspectImage,
    ConvertImage,
    DocumentImage,
    DocumentMermaid,
    DocumentMath {
        display: bool,
    },
    MediaMetadata,
    RawMetadata,
    SearchExtract,
    SearchMediaInfo,
    SearchVideoFrame,
    SearchAudioSegment,
    PreviewWorkbook,
    PreviewDocument,
    PreviewPdf(PdfRenderSize),
    PreviewModel(ModelRender),
    PreviewMedia(MediaPreviewSize),
    PreviewAudio(MediaPreviewSize),
    AudioPeaks,
    VideoStoryboard {
        cell_edge: u32,
    },
    AudioTags,
    AudioCover,
    ArchiveList {
        format: ArchiveFormat,
        password: Option<SecretString>,
    },
}

impl ParseOperation {
    pub(crate) fn argument(&self) -> &'static str {
        match self {
            Self::ThumbnailImage => "thumbnail-image",
            Self::ThumbnailRaw => "thumbnail-raw",
            Self::ThumbnailRawLegacy => "thumbnail-raw-legacy",
            Self::ThumbnailPdf => "thumbnail-pdf",
            Self::ThumbnailVideo => "thumbnail-video",
            Self::ThumbnailAppImage => "thumbnail-appimage",
            Self::ThumbnailModel(_) => "thumbnail-model",
            Self::ThumbnailCover(_) => "thumbnail-cover",
            Self::PreviewCover(_) => "preview-cover",
            Self::PreviewImage => "preview-image-rgba",
            Self::PreviewImageLegacy => "preview-image-legacy",
            Self::PreviewRawDevelop => "preview-raw-develop",
            Self::InspectImage => "inspect-image",
            Self::ConvertImage => "convert-image",
            Self::DocumentImage => "document-image",
            Self::DocumentMermaid => "document-mermaid",
            Self::DocumentMath { display: true } => "document-math",
            Self::DocumentMath { display: false } => "document-inline-math",
            Self::MediaMetadata => "media-metadata",
            Self::RawMetadata => "raw-metadata",
            Self::SearchExtract => "search-extract",
            Self::SearchMediaInfo => "search-media-info",
            Self::SearchVideoFrame => "search-video-frame",
            Self::SearchAudioSegment => "search-audio-segment",
            Self::PreviewWorkbook => "preview-workbook",
            Self::PreviewDocument => "preview-document",
            Self::PreviewPdf(_) => "preview-pdf",
            Self::PreviewModel(_) => "preview-model",
            Self::PreviewMedia(_) => "preview-media",
            Self::PreviewAudio(_) => "preview-audio",
            Self::AudioPeaks => "audio-peaks",
            Self::VideoStoryboard { .. } => "video-storyboard",
            Self::AudioTags => "audio-tags",
            Self::AudioCover => "audio-cover",
            Self::ArchiveList { .. } => "archive-list",
        }
    }

    pub(crate) fn output_name(&self) -> &'static str {
        if matches!(
            self,
            Self::MediaMetadata
                | Self::AudioTags
                | Self::RawMetadata
                | Self::SearchExtract
                | Self::PreviewWorkbook
                | Self::PreviewDocument
                | Self::InspectImage
        ) {
            "result.json"
        } else if self.is_media() {
            "result.media"
        } else if matches!(self, Self::ArchiveList { .. }) {
            "result.archive.json"
        } else {
            "result.png"
        }
    }

    pub(crate) fn is_media(&self) -> bool {
        matches!(
            self,
            Self::PreviewMedia(_)
                | Self::PreviewAudio(_)
                | Self::AudioPeaks
                | Self::VideoStoryboard { .. }
        )
    }

    pub(crate) fn input_size_limit(&self) -> Option<u64> {
        match self {
            Self::ThumbnailImage
            | Self::ThumbnailRaw
            | Self::ThumbnailRawLegacy
            | Self::ThumbnailPdf
            | Self::PreviewImage
            | Self::PreviewImageLegacy
            | Self::PreviewRawDevelop
            | Self::RawMetadata
            | Self::SearchExtract
            | Self::PreviewPdf(_) => Some(MAX_RASTER_INPUT_BYTES),
            Self::PreviewModel(_) | Self::ThumbnailModel(_) => Some(MAX_MODEL_INPUT_BYTES),
            Self::ThumbnailCover(_) | Self::PreviewCover(_) => {
                Some(crate::sandbox_helper::archive_cover::MAX_INPUT_BYTES)
            }
            Self::InspectImage | Self::ConvertImage => {
                Some(crate::services::image_conversion::MAX_INPUT_BYTES)
            }
            Self::PreviewWorkbook => Some(crate::services::table::WORKBOOK_BYTE_LIMIT),
            Self::PreviewDocument => Some(crate::services::docx::DOCX_BYTE_LIMIT),
            Self::DocumentImage => Some(crate::services::document_media::IMAGE_INPUT_LIMIT),
            Self::DocumentMermaid => {
                Some(crate::services::document_media::DIAGRAM_INPUT_LIMIT as u64)
            }
            Self::DocumentMath { .. } => {
                Some(crate::services::document_media::MATH_INPUT_LIMIT as u64)
            }
            Self::ThumbnailVideo
            | Self::ThumbnailAppImage
            | Self::PreviewMedia(_)
            | Self::PreviewAudio(_)
            | Self::AudioPeaks
            | Self::VideoStoryboard { .. }
            | Self::AudioTags
            | Self::AudioCover
            | Self::MediaMetadata
            | Self::SearchMediaInfo
            | Self::SearchVideoFrame
            | Self::SearchAudioSegment
            | Self::ArchiveList { .. } => None,
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct Cancellation(pub(crate) Arc<AtomicBool>);

impl From<Arc<AtomicBool>> for Cancellation {
    fn from(flag: Arc<AtomicBool>) -> Self {
        Self(flag)
    }
}

impl Cancellation {
    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub(crate) struct ParseOutput {
    pub(crate) data: Vec<u8>,
    pub(crate) page: i32,
    pub(crate) pages: i32,
    pub(crate) text_layer: Option<crate::services::PdfTextLayer>,
    pub(crate) raster_dimensions: Option<(u32, u32)>,
}

struct PrivateOutput(PathBuf);

impl PrivateOutput {
    fn create() -> std::io::Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "strata-preview-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for PrivateOutput {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn read_private_output(path: &Path, max_bytes: u64) -> std::io::Result<Vec<u8>> {
    let mut file = fs::File::open(path)?;
    let mut data = Vec::new();
    (&mut file).take(max_bytes + 1).read_to_end(&mut data)?;
    if data.len() as u64 > max_bytes {
        return Err(std::io::Error::new(
            std::io::ErrorKind::FileTooLarge,
            "Preview output exceeded the maximum size limit",
        ));
    }
    Ok(data)
}

fn read_meta(path: &Path) -> (i32, i32) {
    let Ok(bytes) = read_private_output(path, 256) else {
        return (0, 0);
    };
    let Ok(value) = std::str::from_utf8(&bytes) else {
        return (0, 0);
    };
    let mut values = value
        .split_whitespace()
        .filter_map(|part| part.parse().ok());
    (values.next().unwrap_or(0), values.next().unwrap_or(0))
}

fn read_text_layer(path: &Path) -> Option<crate::services::PdfTextLayer> {
    let bytes = read_private_output(path, MAX_TEXT_LAYER_BYTES).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn valid_output(operation: ParseOperation, data: &[u8]) -> bool {
    if matches!(operation, ParseOperation::SearchExtract) {
        return data.len() <= 1024 * 1024
            && serde_json::from_slice::<crate::services::smart_search::result::Extraction>(data)
                .is_ok_and(|value| {
                    value.segments.len() <= 1024
                        && value
                            .segments
                            .iter()
                            .all(|segment| segment.text.len() <= 16000)
                });
    }
    if matches!(operation, ParseOperation::SearchMediaInfo) {
        return data.len() <= 64 * 1024
            && serde_json::from_slice::<serde_json::Value>(data).is_ok();
    }
    if matches!(operation, ParseOperation::SearchAudioSegment) {
        return data.len() <= 1024 * 1024
            && data.get(..4) == Some(b"RIFF")
            && data.get(8..12) == Some(b"WAVE");
    }
    if matches!(operation, ParseOperation::ThumbnailRaw) && data.starts_with(b"STRGBA01") {
        return pixels::dimensions(data)
            .is_some_and(|(width, height)| width <= 256 && height <= 256);
    }
    if matches!(
        operation,
        ParseOperation::MediaMetadata
            | ParseOperation::AudioTags
            | ParseOperation::RawMetadata
            | ParseOperation::PreviewWorkbook
            | ParseOperation::PreviewDocument
            | ParseOperation::InspectImage
    ) {
        serde_json::from_slice::<serde_json::Value>(data).is_ok()
    } else if matches!(operation, ParseOperation::ArchiveList { .. }) {
        crate::adapters::archive_payload_valid(data)
    } else if matches!(
        operation,
        ParseOperation::PreviewImage
            | ParseOperation::PreviewImageLegacy
            | ParseOperation::PreviewRawDevelop
    ) {
        rgba_dimensions(data).is_some()
    } else {
        png_dimensions(data).is_some()
    }
}

fn is_archive_contract_message(message: &str) -> bool {
    message == crate::adapters::INVALID_ARCHIVE
        || message == crate::adapters::ARCHIVE_TOO_LARGE_MESSAGE
        || message == "Preview cancelled"
}

pub(crate) fn parse(
    input: &Path,
    operation: ParseOperation,
    value: i32,
    media_backend: MediaPreviewBackend,
    cancellation: &Cancellation,
) -> Result<ParseOutput, String> {
    parse_with_progress(
        input,
        operation,
        value,
        media_backend,
        cancellation,
        &|_| {},
    )
}

pub(crate) fn parse_with_progress(
    input: &Path,
    operation: ParseOperation,
    value: i32,
    media_backend: MediaPreviewBackend,
    cancellation: &Cancellation,
    progress: &dyn Fn(crate::services::ModelPreviewStage),
) -> Result<ParseOutput, String> {
    let archive = matches!(operation, ParseOperation::ArchiveList { .. });
    let result = parse_sandboxed(
        input,
        operation,
        value,
        media_backend,
        cancellation,
        progress,
    );
    match result {
        Err(error)
            if archive && !cancellation.is_cancelled() && !is_archive_contract_message(&error) =>
        {
            Err(crate::adapters::ARCHIVE_PREVIEW_FAILED_MESSAGE.to_owned())
        }
        result => result,
    }
}

fn resolve_executable() -> Result<PathBuf, String> {
    let current = std::env::current_exe()
        .map_err(|error| format!("Unable to locate the Strata executable: {error}"))?;

    if let Some(parent) = current.parent() {
        if parent.file_name().is_some_and(|name| name == "deps") {
            if let Some(grandparent) = parent.parent() {
                let candidate = grandparent.join("strata.exe");
                if candidate.is_file() {
                    return Ok(candidate);
                }
            }
        }
    }

    Ok(current)
}

fn parse_sandboxed(
    input: &Path,
    operation: ParseOperation,
    value: i32,
    media_backend: MediaPreviewBackend,
    cancellation: &Cancellation,
    _progress: &dyn Fn(crate::services::ModelPreviewStage),
) -> Result<ParseOutput, String> {
    if cancellation.is_cancelled() {
        return Err("Preview cancelled".to_owned());
    }
    if operation.is_media() {
        return Err("Media previews require a decoded-frame session".into());
    }
    let _lease = crate::services::file_mutations::worker(input, cancellation.0.clone());
    let metadata = fs::metadata(input).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("Preview input is not a regular file".to_owned());
    }
    if operation
        .input_size_limit()
        .is_some_and(|limit| metadata.len() > limit)
    {
        return Err(
            if matches!(
                operation,
                ParseOperation::InspectImage | ParseOperation::ConvertImage
            ) {
                "The image exceeds the 32 MiB conversion limit. Choose a smaller image.".to_owned()
            } else if matches!(operation, ParseOperation::PreviewModel(_)) {
                "This model file exceeds the 128 MiB preview limit. Try a smaller or lower-detail version.".to_owned()
            } else {
                "Preview input exceeds the supported size limit".to_owned()
            },
        );
    }

    if let Some(result) = browser::preview_rgba(input, &operation, cancellation) {
        return result.map(|((width, height), data)| ParseOutput {
            data,
            page: 0,
            pages: 0,
            text_layer: None,
            raster_dimensions: Some((width, height)),
        });
    }

    let output =
        PrivateOutput::create().map_err(|error| crate::services::io_error_message(&error))?;
    let secret = if let ParseOperation::ArchiveList {
        password: Some(password),
        ..
    } = &operation
    {
        Some(password.expose().as_bytes().to_vec())
    } else {
        None
    };

    let current_executable = resolve_executable()?;

    let value_str = match &operation {
        ParseOperation::PreviewPdf(size) => {
            let size = PdfRenderSize::new(size.width, size.height);
            format!("{value}:{}x{}", size.width, size.height)
        }
        ParseOperation::PreviewModel(render) => format!(
            "{}:{}x{}:{:06x}:{:06x}",
            render.format.argument(),
            render.size.width,
            render.size.height,
            render.palette.accent,
            render.palette.surface,
        ),
        ParseOperation::ThumbnailModel(format) => format.argument().to_owned(),
        ParseOperation::ThumbnailCover(format) => format.argument().to_owned(),
        ParseOperation::PreviewCover(format) => format.argument().to_owned(),
        ParseOperation::ArchiveList { format, .. } => format.extension().to_owned(),
        _ => value.to_string(),
    };

    let output_file = output.path().join(operation.output_name());
    let mut args = vec![
        std::ffi::OsString::from("--preview-helper"),
        std::ffi::OsString::from(operation.argument()),
        input.as_os_str().to_os_string(),
        output_file.as_os_str().to_os_string(),
        std::ffi::OsString::from(value_str),
        std::ffi::OsString::from(media_backend.argument()),
    ];
    if secret.is_some() {
        args.push(std::ffi::OsString::from("0"));
    }

    let limits = strata_win_shell::job::JobLimits {
        memory_limit_bytes: Some(1024 * 1024 * 1024),
        active_process_limit: Some(
            if matches!(
                operation,
                ParseOperation::SearchMediaInfo
                    | ParseOperation::SearchVideoFrame
                    | ParseOperation::SearchAudioSegment
            ) {
                2
            } else {
                1
            },
        ),
        kill_on_job_close: true,
        restrict_ui: true,
    };

    let arg_refs: Vec<&std::ffi::OsStr> = args.iter().map(|a| a.as_os_str()).collect();
    let timeout = if matches!(
        operation,
        ParseOperation::DocumentImage
            | ParseOperation::DocumentMermaid
            | ParseOperation::DocumentMath { .. }
    ) {
        Duration::from_secs(3)
    } else {
        WALL_TIME_LIMIT
    };

    let job_result = strata_win_shell::job::run_in_job(
        &current_executable,
        &arg_refs,
        secret.as_deref(),
        &limits,
        timeout,
        Some(&cancellation.0),
    );

    let job_output = match job_result {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
            return Err("Preview cancelled".to_owned());
        }
        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
            return Err("The sandboxed preview worker exceeded its time limit".to_owned());
        }
        Err(e) => {
            return Err(format!("The sandboxed preview worker failed to start: {e}"));
        }
    };

    if job_output.exit_code != 0 {
        if let Ok(error_bytes) = fs::read(output.path().join("result.error"))
            && let Ok(error_msg) = String::from_utf8(error_bytes)
        {
            return Err(error_msg);
        }
        return Err("The sandboxed preview renderer failed".to_owned());
    }

    let limit = if matches!(operation, ParseOperation::InspectImage) {
        32
    } else if matches!(
        operation,
        ParseOperation::MediaMetadata | ParseOperation::AudioTags | ParseOperation::RawMetadata
    ) {
        crate::sandbox::metadata::MAX_METADATA_BYTES
    } else {
        MAX_OUTPUT_BYTES
    };

    let data = read_private_output(&output_file, limit).map_err(|error| {
        if matches!(operation, ParseOperation::ArchiveList { .. }) {
            crate::adapters::ARCHIVE_PREVIEW_FAILED_MESSAGE.to_owned()
        } else {
            error.to_string()
        }
    })?;

    if !valid_output(operation.clone(), &data) {
        return Err(if matches!(operation, ParseOperation::ArchiveList { .. }) {
            crate::adapters::INVALID_ARCHIVE.to_owned()
        } else {
            "The preview renderer returned corrupted data".to_owned()
        });
    }

    let (page, pages) = read_meta(&output.path().join("result.meta"));
    let text_layer = read_text_layer(&output.path().join("result.text"));

    let (data, raster_dimensions) = if matches!(
        operation,
        ParseOperation::PreviewImage
            | ParseOperation::PreviewImageLegacy
            | ParseOperation::PreviewRawDevelop
    ) {
        let (dimensions, pixels) = decode_rgba_output(&data)?;
        (pixels, Some(dimensions))
    } else {
        (data, None)
    };

    Ok(ParseOutput {
        data,
        page,
        pages,
        text_layer,
        raster_dimensions,
    })
}

pub(crate) fn sandbox_unavailable(detail: &str) -> String {
    format!("Secure Windows preview unavailable: {detail}")
}

pub(crate) fn sandbox_start_failed(error: &std::io::Error) -> String {
    sandbox_unavailable(&error.to_string())
}

pub(crate) fn polaris_gpu_available() -> bool {
    false
}

pub(crate) fn gpu_devices(_root: &Path, _backend: MediaPreviewBackend) -> Vec<PathBuf> {
    Vec::new()
}

pub(crate) fn numbered_name(name: &std::ffi::OsStr, prefix: &str) -> bool {
    name.to_str()
        .and_then(|name| name.strip_prefix(prefix))
        .is_some_and(|suffix| {
            !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
        })
}

pub(crate) fn png_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    if !data.starts_with(b"\x89PNG\r\n\x1a\n")
        || data.get(8..12)? != 13u32.to_be_bytes()
        || data.get(12..16)? != b"IHDR"
    {
        return None;
    }
    let width = u32::from_be_bytes(data.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(data.get(20..24)?.try_into().ok()?);
    (width > 0 && height > 0).then_some((width, height))
}

fn rgba_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    pixels::dimensions(data)
}

fn decode_rgba_output(data: &[u8]) -> Result<((u32, u32), Vec<u8>), String> {
    let dimensions =
        rgba_dimensions(data).ok_or("The preview renderer returned invalid RGBA data")?;
    Ok((dimensions, data[16..].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_preview_image_returns_rgba_without_png_roundtrip() {
        let dir = tempfile::tempdir().expect("image fixture directory");
        let path = dir.path().join("sample.png");
        let expected = [15, 30, 45, 255];
        image::RgbaImage::from_pixel(2, 1, image::Rgba(expected))
            .save(&path)
            .expect("write fixture");
        let output = parse(
            &path,
            ParseOperation::PreviewImage,
            0,
            MediaPreviewBackend::Software,
            &Cancellation::default(),
        )
        .expect("decode image once");
        assert_eq!(output.raster_dimensions, Some((2, 1)));
        assert_eq!(output.data, [expected, expected].concat());
        assert!(!output.data.starts_with(b"\x89PNG"));
    }

    #[test]
    fn untrusted_parse_refused_without_touching_path() {
        let c = Cancellation::default();
        let file = Path::new("Z:\\offline-not-accessible\\data.pdf");
        assert!(
            parse(
                file,
                ParseOperation::ThumbnailPdf,
                0,
                MediaPreviewBackend::Software,
                &c
            )
            .is_err()
        );
        c.cancel();
        let error = parse(
            file,
            ParseOperation::ThumbnailPdf,
            0,
            MediaPreviewBackend::Software,
            &c,
        )
        .err()
        .expect("cancellation wins");
        assert!(error.contains("cancelled"));
    }

    #[test]
    fn sandboxed_thumbnail_image_via_job() {
        let dir = tempfile::tempdir().expect("tempdir");
        let img_path = dir.path().join("test.png");
        image::RgbaImage::from_pixel(10, 10, image::Rgba([100, 150, 200, 255]))
            .save(&img_path)
            .expect("save png");

        let output = parse(
            &img_path,
            ParseOperation::ThumbnailImage,
            64,
            MediaPreviewBackend::Software,
            &Cancellation::default(),
        )
        .expect("sandboxed thumbnail");

        assert!(png_dimensions(&output.data).is_some());
    }

    #[test]
    fn sandboxed_archive_list_parses_zip() {
        use std::io::Write;
        let dir = tempfile::tempdir().expect("tempdir");
        let zip_path = dir.path().join("sample.zip");
        let file = fs::File::create(&zip_path).expect("create zip");
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("hello.txt", zip::write::SimpleFileOptions::default())
            .expect("start file");
        zip.write_all(b"Hello from inside zip")
            .expect("write inside zip");
        zip.finish().expect("finish zip");

        let output = parse(
            &zip_path,
            ParseOperation::ArchiveList {
                format: ArchiveFormat::Zip,
                password: None,
            },
            0,
            MediaPreviewBackend::Software,
            &Cancellation::default(),
        )
        .expect("parse sandboxed zip");

        assert!(crate::adapters::archive_payload_valid(&output.data));
    }

    #[test]
    fn job_object_creation_and_limits() {
        let limits = strata_win_shell::JobLimits::default();
        let job = strata_win_shell::JobObject::create(&limits).expect("create job object");
        assert!(job.is_valid());
    }

    #[test]
    fn run_in_job_executes_and_terminates() {
        let limits = strata_win_shell::JobLimits::default();
        let cmd = std::path::Path::new("cmd.exe");
        let args = [
            std::ffi::OsStr::new("/c"),
            std::ffi::OsStr::new("echo hello sandbox"),
        ];
        let output =
            strata_win_shell::run_in_job(cmd, &args, None, &limits, Duration::from_secs(5), None)
                .expect("run cmd in job");

        assert_eq!(output.exit_code, 0);
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(text.contains("hello sandbox"), "got output: {text}");
    }

    #[test]
    fn run_in_job_respects_cancellation() {
        let limits = strata_win_shell::JobLimits::default();
        let cmd = std::path::Path::new("cmd.exe");
        let args = [
            std::ffi::OsStr::new("/c"),
            std::ffi::OsStr::new("timeout 10"),
        ];
        let cancelled = std::sync::atomic::AtomicBool::new(true);
        let result = strata_win_shell::run_in_job(
            cmd,
            &args,
            None,
            &limits,
            Duration::from_secs(5),
            Some(&cancelled),
        );
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::Interrupted);
    }
}
