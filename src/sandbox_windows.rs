// SPDX-License-Identifier: MIT
//! Native Windows sandbox boundary. Linux bubblewrap, seccomp, Landlock and
//! Unix process-group termination are not available on Win32. NEVER run an
//! untrusted thumbnail, media, archive, PDF or model parser in-process as a
//! workaround. A bounded, ACL-isolated Windows process + Job Object is required
//! before any of these operations may be enabled.
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::services::{
    ArchiveFormat, MediaPreviewSize, ModelFormat, ModelRender, SecretString,
};

#[path = "sandbox/browser_windows.rs"]
pub(crate) mod browser;
#[path = "sandbox/media_windows.rs"]
pub(crate) mod media;
#[path = "sandbox/metadata.rs"]
pub(crate) mod metadata;
#[path = "sandbox/raw_metadata.rs"]
pub(crate) mod raw_metadata;

pub(crate) const FILE_SIZE_LIMIT_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const MAX_OUTPUT_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const MAX_TEXT_LAYER_BYTES: u64 = 8 * 1024 * 1024;

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
    ThumbnailPdf,
    ThumbnailVideo,
    ThumbnailAppImage,
    ThumbnailModel(ModelFormat),
    ThumbnailCover(CoverFormat),
    PreviewCover(CoverFormat),
    PreviewImage,
    InspectImage,
    ConvertImage,
    DocumentImage,
    DocumentMermaid,
    DocumentMath {
        display: bool,
    },
    MediaMetadata,
    RawMetadata,
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

#[derive(Clone, Default)]
pub(crate) struct Cancellation(Arc<AtomicBool>);

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
}


const UNSUPPORTED: &str =
    "Secure preview sandbox on Windows is not implemented; no untrusted decoder was started";

/// Fail closed without reading a file or spawning a child. Callers already
/// execute preview work on independent background workers.
pub(crate) fn parse(
    _input: &Path,
    _operation: ParseOperation,
    _value: i32,
    _media_backend: MediaPreviewBackend,
    cancellation: &Cancellation,
) -> Result<ParseOutput, String> {
    if cancellation.is_cancelled() {
        Err("Preview cancelled".to_owned())
    } else {
        Err(UNSUPPORTED.to_owned())
    }
}

pub(crate) fn parse_with_progress(
    input: &Path,
    operation: ParseOperation,
    value: i32,
    backend: MediaPreviewBackend,
    cancellation: &Cancellation,
    _progress: &dyn Fn(crate::services::ModelPreviewStage),
) -> Result<ParseOutput, String> {
    parse(input, operation, value, backend, cancellation)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untrusted_parse_refused_without_touching_path() {
        let c = Cancellation::default();
        let file = Path::new("Z:\\offline-not-accessible\\data.pdf");
        assert!(parse(
            file,
            ParseOperation::ThumbnailPdf,
            0,
            MediaPreviewBackend::Software,
            &c
        ).is_err());
        c.cancel();
        let error = parse(file, ParseOperation::ThumbnailPdf, 0, MediaPreviewBackend::Software, &c)
            .expect_err("cancellation wins");
        assert!(error.contains("cancelled"));
    }
}
