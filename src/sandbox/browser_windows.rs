// SPDX-License-Identifier: MIT
//! Windows browser thumbnail workers.
//!
//! Image decoders remain inside a Win32 Job Object, but the worker process is
//! reused for multiple requests. Starting a fresh Strata process for every
//! visible tile made a large directory pay GTK/runtime startup costs hundreds
//! of times before the first row was complete.

use std::{
    ffi::{OsStr, OsString},
    io::{self, Read, Write},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    sync::{Condvar, Mutex, OnceLock},
    time::{Duration, Instant},
};

use super::metadata::MediaMetadata;
use super::{Cancellation, ParseOperation};

pub(crate) const MAX_WORKERS: usize = 16;
const PERSISTENT_WORKERS: usize = 4;
const REQUEST_MAGIC: &[u8; 4] = b"STW1";
const RESPONSE_MAGIC: &[u8; 4] = b"STR1";
const MAX_REQUEST_FIELD: usize = 16 * 1024;
const MAX_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
const PREVIEW_WORKERS: usize = 1;
const VIDEO_WORKERS: usize = 2;
const WORKER_LIMITS: strata_win_shell::job::JobLimits = strata_win_shell::job::JobLimits {
    memory_limit_bytes: Some(1024 * 1024 * 1024),
    active_process_limit: Some(1),
    kill_on_job_close: true,
    restrict_ui: true,
};
// Video thumbnails use one bounded ffmpeg child inside the decoder worker.
// Keeping the active-process limit at two still prevents a decoder from
// creating an unbounded process tree while allowing the required media tool.
const VIDEO_WORKER_LIMITS: strata_win_shell::job::JobLimits = strata_win_shell::job::JobLimits {
    memory_limit_bytes: Some(1024 * 1024 * 1024),
    active_process_limit: Some(2),
    kill_on_job_close: true,
    restrict_ui: true,
};

pub(crate) fn default_worker_limit() -> usize {
    PERSISTENT_WORKERS
}

pub(crate) fn worker_limit() -> usize {
    PERSISTENT_WORKERS
}

pub(crate) fn set_worker_limit(_limit: usize) {}

pub(crate) struct Thumbnail {
    pub(crate) png: Vec<u8>,
    pub(crate) metadata: Option<MediaMetadata>,
}

type RgbaPreview = ((u32, u32), Vec<u8>);

struct Worker {
    child: strata_win_shell::job::InteractiveSandboxedChild,
    completed_requests: usize,
}

struct WorkerPool {
    state: Mutex<PoolState>,
    changed: Condvar,
    limit: usize,
    limits: &'static strata_win_shell::job::JobLimits,
}

const WORKER_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_WORKER_REQUESTS: usize = 128;
const MAX_REUSABLE_WORKER_BYTES: usize = 256 * 1024 * 1024;

struct PoolState {
    idle: Vec<(Worker, Instant)>,
    count: usize,
    reaper_running: bool,
}

impl WorkerPool {
    fn acquire(&'static self, cancellation: &Cancellation) -> Result<Worker, String> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        loop {
            let expired = Self::take_expired_idle(&mut state, Instant::now());
            if !expired.is_empty() {
                drop(state);
                drop(expired);
                state = self
                    .state
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
            }
            if cancellation.is_cancelled() {
                return Err("Preview cancelled".to_owned());
            }
            if let Some((worker, _)) = state.idle.pop() {
                return Ok(worker);
            }
            if state.count < self.limit {
                state.count += 1;
                drop(state);
                let worker = Worker::spawn(self.limits);
                if worker.is_err() {
                    let mut state = self
                        .state
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner());
                    state.count = state.count.saturating_sub(1);
                    self.changed.notify_one();
                }
                return worker;
            }
            state = self
                .changed
                .wait_timeout(state, Duration::from_millis(20))
                .map(|(state, _)| state)
                .unwrap_or_else(|poison| poison.into_inner().0);
        }
    }

    fn release(&'static self, worker: Worker, healthy: bool) {
        // Recycling only after the response keeps active previews intact and
        // releases large decoder heaps instead of merely paging them out.
        let healthy = healthy
            && worker.completed_requests < MAX_WORKER_REQUESTS
            && worker
                .child
                .peak_process_memory()
                .is_ok_and(|bytes| bytes < MAX_REUSABLE_WORKER_BYTES);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let mut spawn_reaper = false;
        let retired = if healthy {
            state.idle.push((worker, Instant::now()));
            if !state.reaper_running {
                state.reaper_running = true;
                spawn_reaper = true;
            }
            None
        } else {
            state.count = state.count.saturating_sub(1);
            Some(worker)
        };
        self.changed.notify_one();
        drop(state);
        drop(retired);
        if spawn_reaper {
            let _ = std::thread::Builder::new()
                .name("strata-worker-reaper".to_owned())
                .spawn(move || self.reap_idle_loop());
        }
    }

    fn take_expired_idle(state: &mut PoolState, now: Instant) -> Vec<Worker> {
        let mut expired = Vec::new();
        let mut index = 0;
        while index < state.idle.len() {
            if now.saturating_duration_since(state.idle[index].1) >= WORKER_IDLE_TIMEOUT {
                let (worker, _) = state.idle.swap_remove(index);
                state.count = state.count.saturating_sub(1);
                expired.push(worker);
            } else {
                index += 1;
            }
        }
        expired
    }

    fn reap_idle_loop(&'static self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        loop {
            let now = Instant::now();
            let expired = Self::take_expired_idle(&mut state, now);
            if !expired.is_empty() {
                self.changed.notify_all();
                drop(state);
                drop(expired);
                state = self
                    .state
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                continue;
            }
            let Some(next_wait) = state
                .idle
                .iter()
                .map(|(_, idle_since)| {
                    WORKER_IDLE_TIMEOUT.saturating_sub(now.saturating_duration_since(*idle_since))
                })
                .min()
            else {
                state.reaper_running = false;
                return;
            };
            state = self
                .changed
                .wait_timeout(state, next_wait.max(Duration::from_millis(50)))
                .map(|(state, _)| state)
                .unwrap_or_else(|poison| poison.into_inner().0);
        }
    }
}

fn pool() -> &'static WorkerPool {
    static POOL: OnceLock<WorkerPool> = OnceLock::new();
    POOL.get_or_init(|| WorkerPool {
        state: Mutex::new(PoolState {
            idle: Vec::new(),
            count: 0,
            reaper_running: false,
        }),
        changed: Condvar::new(),
        limit: PERSISTENT_WORKERS,
        limits: &WORKER_LIMITS,
    })
}

fn preview_pool() -> &'static WorkerPool {
    static POOL: OnceLock<WorkerPool> = OnceLock::new();
    POOL.get_or_init(|| WorkerPool {
        state: Mutex::new(PoolState {
            idle: Vec::new(),
            count: 0,
            reaper_running: false,
        }),
        changed: Condvar::new(),
        limit: PREVIEW_WORKERS,
        limits: &WORKER_LIMITS,
    })
}

fn video_pool() -> &'static WorkerPool {
    static POOL: OnceLock<WorkerPool> = OnceLock::new();
    POOL.get_or_init(|| WorkerPool {
        state: Mutex::new(PoolState {
            idle: Vec::new(),
            count: 0,
            reaper_running: false,
        }),
        changed: Condvar::new(),
        limit: VIDEO_WORKERS,
        limits: &VIDEO_WORKER_LIMITS,
    })
}

impl Worker {
    fn spawn(limits: &'static strata_win_shell::job::JobLimits) -> Result<Self, String> {
        let executable = super::resolve_executable()?;
        let args = [OsStr::new("--browser-worker")];
        let child = strata_win_shell::job::spawn_interactive_in_job(&executable, &args, limits)
            .map_err(|error| format!("Unable to start the persistent thumbnail worker: {error}"))?;
        Ok(Self {
            child,
            completed_requests: 0,
        })
    }

    fn execute(
        &mut self,
        path: &Path,
        operation: &ParseOperation,
        cancellation: &Cancellation,
    ) -> Result<Vec<u8>, String> {
        self.child
            .write_stdin(&encode_request(path, operation))
            .map_err(|error| format!("Thumbnail worker write failed: {error}"))?;

        let deadline = Instant::now() + super::WALL_TIME_LIMIT;
        let mut header = [0u8; 9];
        self.child
            .read_stdout_exact(&mut header, deadline, Some(&cancellation.0))
            .map_err(|error| {
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::TimedOut
                ) {
                    return format_worker_io(error);
                }
                let stderr_bytes = self.child.stderr();
                let stderr = String::from_utf8_lossy(&stderr_bytes);
                if stderr.is_empty() {
                    format_worker_io(error)
                } else {
                    format!("{}: {}", format_worker_io(error), stderr.trim())
                }
            })?;
        if &header[..4] != RESPONSE_MAGIC {
            return Err("Thumbnail worker returned an invalid response header".to_owned());
        }
        let status = header[4];
        let length = u32::from_le_bytes(header[5..9].try_into().expect("response length")) as usize;
        let limit = if status == 0 {
            MAX_OUTPUT_BYTES
        } else {
            16 * 1024
        };
        if length > limit {
            return Err("Thumbnail worker response exceeded the output limit".to_owned());
        }
        let mut payload = vec![0u8; length];
        self.child
            .read_stdout_exact(&mut payload, deadline, Some(&cancellation.0))
            .map_err(format_worker_io)?;
        if status == 0 {
            if !super::valid_output(operation.clone(), &payload) {
                return Err("Thumbnail worker returned invalid image data".to_owned());
            }
            self.completed_requests = self.completed_requests.saturating_add(1);
            Ok(payload)
        } else {
            Err(String::from_utf8(payload)
                .unwrap_or_else(|_| "Thumbnail worker rejected the input".to_owned()))
        }
    }
}

fn format_worker_io(error: io::Error) -> String {
    match error.kind() {
        io::ErrorKind::Interrupted => "Preview cancelled".to_owned(),
        io::ErrorKind::TimedOut => {
            "The sandboxed preview worker exceeded its time limit".to_owned()
        }
        _ => format!("Thumbnail worker failed: {error}"),
    }
}

pub(crate) fn thumbnail(
    path: &Path,
    operation: ParseOperation,
    cancellation: &Cancellation,
) -> Result<Thumbnail, String> {
    if cancellation.is_cancelled() {
        return Err("Preview cancelled".into());
    }
    let workers = if operation == ParseOperation::ThumbnailVideo {
        video_pool()
    } else {
        pool()
    };
    let mut worker = workers.acquire(cancellation)?;
    let result = worker.execute(path, &operation, cancellation);
    let healthy = result.is_ok();
    workers.release(worker, healthy);
    result.map(|png| Thumbnail {
        png,
        metadata: None,
    })
}

pub(crate) fn preview_rgba(
    path: &Path,
    operation: &ParseOperation,
    cancellation: &Cancellation,
) -> Option<Result<RgbaPreview, String>> {
    if !matches!(
        operation,
        ParseOperation::PreviewImage
            | ParseOperation::PreviewImageLegacy
            | ParseOperation::PreviewRawDevelop
    ) {
        return None;
    }
    if cancellation.is_cancelled() {
        return Some(Err("Preview cancelled".to_owned()));
    }
    let mut worker = match preview_pool().acquire(cancellation) {
        Ok(worker) => worker,
        Err(error) => return Some(Err(error)),
    };
    let result = worker
        .execute(path, operation, cancellation)
        .and_then(|envelope| super::decode_rgba_output(&envelope));
    let healthy = result.is_ok();
    preview_pool().release(worker, healthy);
    Some(result)
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

/// The browser-worker mode is entered before GTK application startup. It keeps
/// reading framed requests until the parent closes the pipe or the Job Object
/// terminates it.
pub(crate) fn run() -> Result<(), String> {
    let stdin = io::stdin();
    let mut stdin = stdin.lock();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    loop {
        let Some((path, operation)) = read_request(&mut stdin)? else {
            return Ok(());
        };
        let result = if matches!(
            operation,
            ParseOperation::PreviewImage
                | ParseOperation::PreviewImageLegacy
                | ParseOperation::PreviewRawDevelop
        ) {
            crate::sandbox_helper::render_browser_preview(
                &path,
                &operation,
                &Cancellation::default(),
            )
        } else {
            crate::sandbox_helper::render_browser_thumbnail(
                &path,
                &operation,
                &Cancellation::default(),
            )
        };
        write_response(&mut stdout, result)?;
    }
}

fn encode_request(path: &Path, operation: &ParseOperation) -> Vec<u8> {
    let argument = operation.argument().as_bytes();
    let value = operation_value(operation).as_bytes();
    let path: Vec<u16> = path.as_os_str().encode_wide().collect();
    let mut request = Vec::with_capacity(16 + argument.len() + value.len() + path.len() * 2);
    request.extend_from_slice(REQUEST_MAGIC);
    request.extend_from_slice(&(argument.len() as u32).to_le_bytes());
    request.extend_from_slice(&(value.len() as u32).to_le_bytes());
    request.extend_from_slice(&(path.len() as u32).to_le_bytes());
    request.extend_from_slice(argument);
    request.extend_from_slice(value);
    for unit in path {
        request.extend_from_slice(&unit.to_le_bytes());
    }
    request
}

fn operation_value(operation: &ParseOperation) -> &'static str {
    match operation {
        ParseOperation::ThumbnailModel(format) => format.argument(),
        ParseOperation::ThumbnailCover(format) => format.argument(),
        _ => "256",
    }
}

fn read_request(reader: &mut impl Read) -> Result<Option<(PathBuf, ParseOperation)>, String> {
    let mut header = [0u8; 16];
    match reader.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error.to_string()),
    }
    if &header[..4] != REQUEST_MAGIC {
        return Err("Invalid thumbnail worker request header".to_owned());
    }
    let operation_len =
        u32::from_le_bytes(header[4..8].try_into().expect("operation length")) as usize;
    let value_len = u32::from_le_bytes(header[8..12].try_into().expect("value length")) as usize;
    let path_units = u32::from_le_bytes(header[12..16].try_into().expect("path length")) as usize;
    if operation_len > MAX_REQUEST_FIELD
        || value_len > MAX_REQUEST_FIELD
        || path_units > MAX_REQUEST_FIELD
    {
        return Err("Thumbnail worker request exceeded the input limit".to_owned());
    }
    let mut operation = vec![0u8; operation_len];
    let mut value = vec![0u8; value_len];
    let mut path_bytes = vec![0u8; path_units.saturating_mul(2)];
    reader
        .read_exact(&mut operation)
        .map_err(|error| error.to_string())?;
    reader
        .read_exact(&mut value)
        .map_err(|error| error.to_string())?;
    reader
        .read_exact(&mut path_bytes)
        .map_err(|error| error.to_string())?;
    let operation = std::str::from_utf8(&operation).map_err(|_| "Invalid worker operation")?;
    let value = std::str::from_utf8(&value).map_err(|_| "Invalid worker value")?;
    let path = path_bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|bytes| u16::from_le_bytes(*bytes))
        .collect::<Vec<_>>();
    Ok(Some((
        PathBuf::from(OsString::from_wide(&path)),
        parse_operation(operation, value)?,
    )))
}

fn parse_operation(operation: &str, value: &str) -> Result<ParseOperation, String> {
    Ok(match operation {
        "preview-image-rgba" => ParseOperation::PreviewImage,
        "thumbnail-image" => ParseOperation::ThumbnailImage,
        "thumbnail-raw" => ParseOperation::ThumbnailRaw,
        "thumbnail-raw-legacy" => ParseOperation::ThumbnailRawLegacy,
        "preview-image-legacy" => ParseOperation::PreviewImageLegacy,
        "preview-raw-develop" => ParseOperation::PreviewRawDevelop,
        "thumbnail-pdf" => ParseOperation::ThumbnailPdf,
        "thumbnail-video" => ParseOperation::ThumbnailVideo,
        "thumbnail-appimage" => ParseOperation::ThumbnailAppImage,
        "thumbnail-model" => ParseOperation::ThumbnailModel(
            crate::services::ModelFormat::from_argument(value).ok_or("Invalid model format")?,
        ),
        "thumbnail-cover" => ParseOperation::ThumbnailCover(
            super::CoverFormat::from_argument(value).ok_or("Invalid cover format")?,
        ),
        _ => return Err("Unsupported thumbnail worker operation".to_owned()),
    })
}

fn write_response(writer: &mut impl Write, result: Result<Vec<u8>, String>) -> Result<(), String> {
    let (status, payload) = match result {
        Ok(payload) if payload.len() <= MAX_OUTPUT_BYTES => (0u8, payload),
        Ok(_) => (1u8, b"Thumbnail worker output exceeded the limit".to_vec()),
        Err(error) => (1u8, error.into_bytes()),
    };
    let payload = if payload.len() > 16 * 1024 && status != 0 {
        b"Thumbnail worker rejected the input".to_vec()
    } else {
        payload
    };
    writer
        .write_all(RESPONSE_MAGIC)
        .and_then(|()| writer.write_all(&[status]))
        .and_then(|()| writer.write_all(&(payload.len() as u32).to_le_bytes()))
        .and_then(|()| writer.write_all(&payload))
        .and_then(|()| writer.flush())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_worker_renders_and_reuses_a_job() {
        let directory = tempfile::tempdir().expect("thumbnail fixture directory");
        let path = directory.path().join("sample.png");
        image::RgbaImage::from_pixel(12, 8, image::Rgba([25, 50, 75, 255]))
            .save(&path)
            .expect("write thumbnail fixture");
        let cancellation = Cancellation::default();
        let first = thumbnail(&path, ParseOperation::ThumbnailImage, &cancellation)
            .expect("first persistent worker request");
        let second = thumbnail(&path, ParseOperation::ThumbnailImage, &cancellation)
            .expect("second persistent worker request");
        assert!(!first.png.is_empty());
        assert_eq!(first.png, second.png);
    }

    #[test]
    fn persistent_video_worker_renders_a_thumbnail() {
        let directory = tempfile::tempdir().expect("video fixture directory");
        let path = directory.path().join("sample.mp4");
        let ffmpeg = std::env::var_os("STRATA_FFMPEG")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|executable| {
                        executable.parent().map(|parent| parent.join("ffmpeg.exe"))
                    })
                    .filter(|executable| executable.is_file())
            })
            .unwrap_or_else(|| PathBuf::from("ffmpeg.exe"));
        let output = std::process::Command::new(ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=320x180:r=12:d=1",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-threads",
                "1",
            ])
            .arg(&path)
            .output()
            .expect("ffmpeg must be available for the video thumbnail regression");
        assert!(
            output.status.success(),
            "video fixture encoder failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let thumbnail = thumbnail(
            &path,
            ParseOperation::ThumbnailVideo,
            &Cancellation::default(),
        )
        .expect("persistent video worker thumbnail");
        let image = image::load_from_memory(&thumbnail.png).expect("valid video thumbnail PNG");
        assert!(image.width() > 0 && image.height() > 0);
    }
}
