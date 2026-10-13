// SPDX-License-Identifier: MIT
//! Windows browser thumbnail/metadata helpers. The Linux persistent worker
//! relies on seccomp+Landlock+bubblewrap and Unix sockets. Do not fall back to
//! an unsandboxed process or decode arbitrary files in the GTK process.
use std::path::Path;

use super::{Cancellation, ParseOperation};
use super::metadata::MediaMetadata;

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
    _path: &Path,
    _operation: ParseOperation,
    cancellation: &Cancellation,
) -> Result<Thumbnail, String> {
    if cancellation.is_cancelled() {
        return Err("Preview cancelled".into());
    }
    Err("A secure Windows thumbnail worker is not available".into())
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
