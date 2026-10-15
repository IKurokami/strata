// SPDX-License-Identifier: MIT
//! Windows browser thumbnail and metadata helpers routed through the secure Job Object sandbox.
use std::path::Path;

use super::metadata::MediaMetadata;
use super::{Cancellation, ParseOperation};

pub(crate) const MAX_WORKERS: usize = 16;

pub(crate) fn default_worker_limit() -> usize {
    4
}

pub(crate) fn worker_limit() -> usize {
    4
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
    let output = super::parse(
        path,
        operation,
        256,
        super::MediaPreviewBackend::Software,
        cancellation,
    )?;
    Ok(Thumbnail {
        png: output.data,
        metadata: None,
    })
}

pub(crate) fn metadata(
    path: &Path,
    image: bool,
    cancellation: &Cancellation,
) -> Result<MediaMetadata, String> {
    if cancellation.is_cancelled() {
        return Err("Preview cancelled".into());
    }
    let output = super::parse(
        path,
        ParseOperation::MediaMetadata,
        0,
        super::MediaPreviewBackend::Software,
        cancellation,
    )?;
    MediaMetadata::from_json(&output.data, image)
}

pub(crate) fn preview(
    path: &Path,
    operation: &ParseOperation,
    cancellation: &Cancellation,
) -> Option<Result<Vec<u8>, String>> {
    Some(
        super::parse(
            path,
            operation.clone(),
            0,
            super::MediaPreviewBackend::Software,
            cancellation,
        )
        .map(|out| out.data),
    )
}

/// --browser-worker is a private Linux IPC protocol, NOT a Windows CLI path.
pub(crate) fn run() -> Result<(), String> {
    Err("The Linux sandbox worker cannot run on Windows".into())
}
