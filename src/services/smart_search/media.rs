// SPDX-License-Identifier: MIT

use super::{
    model,
    result::{ContentLocator, ExtractedSegment, FileRevision},
    runtime::Embedder,
    store,
};
use std::{fs, path::Path};

#[expect(
    clippy::too_many_arguments,
    reason = "the bounded media job reuses the indexer's existing owners"
)]
pub(super) fn index(
    path: &Path,
    revision: &FileRevision,
    connection: &mut rusqlite::Connection,
    root: &Path,
    worker: &mut Embedder,
    artifacts: &Path,
    cancellation: &crate::sandbox::Cancellation,
    cancelled: &impl Fn() -> bool,
    video: bool,
    cursor: u32,
) -> Result<bool, String> {
    let output = crate::sandbox::parse(
        path,
        crate::sandbox::ParseOperation::SearchMediaInfo,
        0,
        crate::sandbox::MediaPreviewBackend::Software,
        cancellation,
    )?;
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.data).map_err(|error| error.to_string())?;
    let duration = metadata["format"]["duration"]
        .as_str()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0 && *value <= 86400.0)
        .ok_or("Media duration exceeds the content indexing budget")?;
    let hop = if video { 10_u64 } else { 25 };
    let start_us = u64::from(cursor) * 1_000_000;
    let duration_us = (duration * 1_000_000.0) as u64;
    if start_us >= duration_us {
        return Ok(false);
    }
    let mut segments = Vec::new();
    let mut vectors = Vec::new();
    if video && cursor.is_multiple_of(10) {
        let output = crate::sandbox::parse(
            path,
            crate::sandbox::ParseOperation::SearchVideoFrame,
            (start_us / 1000).min(i32::MAX as u64) as i32,
            crate::sandbox::MediaPreviewBackend::Software,
            cancellation,
        )?;
        let frame = artifacts.join("video-frame.png");
        fs::write(&frame, &output.data).map_err(|error| error.to_string())?;
        let vector = worker
            .embed("image", &[frame.to_string_lossy().into_owned()], cancelled)?
            .pop()
            .flatten();
        segments.push(ExtractedSegment {
            locator: ContentLocator::Video {
                start_us,
                end_us: (start_us + hop * 1_000_000).min(duration_us),
                soundtrack: false,
            },
            text: String::new(),
            modality: "video".into(),
        });
        vectors.push(vector);
    }
    // Audio windows use DigUp's 30-second window / 25-second hop, independent
    // of the visual frame sampling schedule.
    if (!video || cursor.is_multiple_of(25))
        && let Ok(output) = crate::sandbox::parse(
            path,
            crate::sandbox::ParseOperation::SearchAudioSegment,
            (start_us / 1000).min(i32::MAX as u64) as i32,
            crate::sandbox::MediaPreviewBackend::Software,
            cancellation,
        )
    {
        let audio = artifacts.join("audio-window.wav");
        fs::write(&audio, &output.data).map_err(|error| error.to_string())?;
        let vector = worker
            .embed("audio", &[audio.to_string_lossy().into_owned()], cancelled)?
            .pop()
            .flatten();
        let end_us = (start_us + 30_000_000).min(duration_us);
        segments.push(ExtractedSegment {
            locator: if video {
                ContentLocator::Video {
                    start_us,
                    end_us,
                    soundtrack: true,
                }
            } else {
                ContentLocator::Audio { start_us, end_us }
            },
            text: String::new(),
            modality: "audio".into(),
        });
        vectors.push(vector);
    }
    if cancelled()
        || fs::metadata(path)
            .map(|metadata| FileRevision::from_metadata(&metadata) != *revision)
            .unwrap_or(true)
    {
        return Err("Media changed during content indexing".into());
    }
    let next_second = if video {
        ((cursor / 10 + 1) * 10).min((cursor / 25 + 1) * 25)
    } else {
        cursor + 25
    };
    let next = (u64::from(next_second) * 1_000_000 < duration_us).then_some(next_second);
    store::append(
        connection,
        root,
        path,
        revision,
        &segments,
        &vectors,
        Some(model::FINGERPRINT),
        next,
        cursor > 0,
    )?;
    Ok(next.is_some())
}
