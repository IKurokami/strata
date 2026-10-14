// SPDX-License-Identifier: MIT
//! Windows browser thumbnail/metadata helpers. The Linux persistent worker
//! relies on seccomp+Landlock+bubblewrap and Unix sockets. Do not fall back to
//! an unsandboxed process or decode arbitrary files in the GTK process.
use std::{
    io::Cursor,
    path::Path,
    process::{Command, Stdio},
};

use super::metadata::MediaMetadata;
use super::{Cancellation, ParseOperation};

pub(crate) const MAX_WORKERS: usize = 16;

pub(crate) fn default_worker_limit() -> usize {
    1
}
pub(crate) fn worker_limit() -> usize {
    1
}
pub(crate) fn set_worker_limit(_limit: usize) {}

pub(crate) struct Thumbnail {
    pub(crate) png: Vec<u8>,
    pub(crate) metadata: Option<MediaMetadata>,
}

pub(crate) fn thumbnail(
    path: &Path,
    operation: ParseOperation,
    cancellation: &Cancellation,
) -> Result<Thumbnail, String> {
    if cancellation.is_cancelled() {
        return Err("Preview cancelled".into());
    }
    let png = match operation {
        ParseOperation::ThumbnailImage => {
            let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
            let image = image::ImageReader::new(Cursor::new(bytes))
                .with_guessed_format()
                .map_err(|error| error.to_string())?
                .decode()
                .map_err(|error| error.to_string())?
                .thumbnail(256, 256);
            let mut output = Cursor::new(Vec::new());
            image
                .write_to(&mut output, image::ImageFormat::Png)
                .map_err(|error| error.to_string())?;
            output.into_inner()
        }
        ParseOperation::ThumbnailVideo => {
            let ffmpeg = std::env::var_os("STRATA_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
            let output = Command::new(ffmpeg)
                .args(["-v", "error", "-ss", "0", "-i"])
                .arg(path)
                .args([
                    "-frames:v",
                    "1",
                    "-vf",
                    "scale=256:-2",
                    "-f",
                    "image2pipe",
                    "-vcodec",
                    "png",
                    "pipe:1",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output()
                .map_err(|error| format!("Unable to start thumbnail decoder: {error}"))?;
            if !output.status.success() || output.stdout.is_empty() {
                return Err("The thumbnail decoder returned no video frame".into());
            }
            output.stdout
        }
        _ => return Err("Windows thumbnail format is not supported".into()),
    };
    Ok(Thumbnail {
        png,
        metadata: None,
    })
}

pub(crate) fn metadata(
    _path: &Path,
    _image: bool,
    cancellation: &Cancellation,
) -> Result<MediaMetadata, String> {
    if cancellation.is_cancelled() {
        return Err("Preview cancelled".into());
    }
    Err("A secure Windows metadata worker is not available".into())
}

pub(crate) fn preview(
    _path: &Path,
    _operation: &ParseOperation,
    cancellation: &Cancellation,
) -> Option<Result<Vec<u8>, String>> {
    Some(if cancellation.is_cancelled() {
        Err("Preview cancelled".into())
    } else {
        Err("A secure Windows preview worker is not available".into())
    })
}

/// --browser-worker is a private Linux IPC protocol, NOT a Windows CLI path.
pub(crate) fn run() -> Result<(), String> {
    Err("The Linux sandbox worker cannot run on Windows".into())
}
