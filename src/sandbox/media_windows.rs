// SPDX-License-Identifier: MIT
//! Win32 media sessions require verified child-process containment. A bare
//! GStreamer decoder in the main GTK process is not a secure substitute.
use crate::{
    media::{FPS, Frame, Header, MAX_DURATION_US, Packet},
    services::SandboxedMedia,
};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

pub(crate) const MAX_WORKERS: usize = 4;

#[cfg(test)]
#[path = "media_windows/tests.rs"]
pub(crate) mod tests;

pub(crate) enum Event {
    Prepared(Header),
    Packet(Packet),
    Failed(String),
}

/// Bounded Windows media session backed by the helper decoder process.
pub(crate) struct Session {
    cancelled: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Event>,
    worker: Option<thread::JoinHandle<()>>,
    children: Arc<Mutex<Vec<Arc<strata_win_shell::SandboxedChild>>>>,
}

impl Session {
    pub fn start(source: SandboxedMedia, start_tick: u32) -> Result<Self, String> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        let children = Arc::new(Mutex::new(Vec::new()));
        let worker_children = children.clone();
        let (sender, receiver) = mpsc::sync_channel(3);
        let worker = thread::Builder::new()
            .name("windows-media-helper".into())
            .spawn(move || {
                let result = decode_video(source, start_tick, &stop, &sender, &worker_children);
                // Ensure ffmpeg cannot survive a finished, failed, or cancelled preview.
                terminate_children(&worker_children);
                if let Err(error) = result {
                    let _ = send(&sender, Event::Failed(error), &stop);
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            cancelled,
            receiver,
            worker: Some(worker),
            children,
        })
    }
    pub fn receive(&self) -> Option<Event> {
        self.receiver.try_recv().ok()
    }
    pub fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::AcqRel) {
            let children = self.children.clone();
            // Even dropping the final child handle waits for stderr to drain.
            // Keep termination and handle destruction off the GTK event loop.
            let _ = thread::Builder::new()
                .name("media-cancel".into())
                .spawn(move || {
                    kill_children(&children);
                });
        }
    }
    pub fn finished(&self) -> bool {
        self.worker
            .as_ref()
            .is_none_or(thread::JoinHandle::is_finished)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.cancel();
        // The decoder owns a blocking pipe read. Joining here would block the
        // GTK thread while a replaced selection is still draining FFmpeg.
        // Cancellation is observed by the worker and it tears down its child.
        self.worker.take();
    }
}

fn send(
    sender: &mpsc::SyncSender<Event>,
    mut event: Event,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err("Preview cancelled".into());
        }
        match sender.try_send(event) {
            Ok(()) => return Ok(()),
            Err(mpsc::TrySendError::Disconnected(_)) => return Err("Preview closed".into()),
            Err(mpsc::TrySendError::Full(pending)) => event = pending,
        }
        thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn kill_children(children: &Arc<Mutex<Vec<Arc<strata_win_shell::SandboxedChild>>>>) {
    let jobs = children.lock().map(|jobs| jobs.clone()).unwrap_or_default();
    for child in jobs {
        child.terminate();
    }
}

fn terminate_children(children: &Arc<Mutex<Vec<Arc<strata_win_shell::SandboxedChild>>>>) {
    // Called from the worker, never the GTK event loop: reap all subprocesses.
    // Do not hold the shared registry while waiting for process teardown.
    let jobs = children
        .lock()
        .map(|mut jobs| std::mem::take(&mut *jobs))
        .unwrap_or_default();
    for child in jobs {
        child.terminate();
        child.wait();
    }
}

fn spawn_decoder(
    command: &mut Command,
    cancelled: &AtomicBool,
    children: &Arc<Mutex<Vec<Arc<strata_win_shell::SandboxedChild>>>>,
) -> Result<Arc<strata_win_shell::SandboxedChild>, String> {
    if cancelled.load(Ordering::Acquire) {
        return Err("Preview cancelled".into());
    }
    let executable = PathBuf::from(command.get_program());
    let args: Vec<OsString> = command.get_args().map(OsString::from).collect();
    let arg_refs: Vec<&std::ffi::OsStr> = args.iter().map(OsString::as_os_str).collect();
    let limits = strata_win_shell::JobLimits {
        memory_limit_bytes: Some(1024 * 1024 * 1024),
        active_process_limit: Some(1),
        kill_on_job_close: true,
        restrict_ui: true,
    };
    let child = Arc::new(
        strata_win_shell::spawn_streaming_in_job(&executable, &arg_refs, &limits)
            .map_err(|error| format!("Unable to start contained media decoder: {error}"))?,
    );
    children
        .lock()
        .map_err(|_| "Media decoder process lock poisoned".to_owned())?
        .push(child.clone());
    if cancelled.load(Ordering::Acquire) {
        terminate_children(children);
        return Err("Preview cancelled".into());
    }
    Ok(child)
}

fn read_chunk(reader: &strata_win_shell::SandboxedChild, count: usize) -> Result<Vec<u8>, String> {
    let mut bytes = vec![0u8; count];
    let mut read = 0;
    while read < count {
        match reader.read_stdout(&mut bytes[read..]) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("Media decoder pipe failed: {error}")),
        }
    }
    bytes.truncate(read);
    Ok(bytes)
}

fn audio_command(source: &SandboxedMedia, start_tick: u32) -> Command {
    let mut cmd = Command::new(media_tool("STRATA_FFMPEG", "ffmpeg.exe"));
    cmd.arg("-v")
        .arg("error")
        // Input-side seek avoids decoding every preceding second on a scrub.
        .arg("-ss")
        .arg(format!("{:.3}", f64::from(start_tick) / f64::from(FPS)))
        .arg("-i")
        .arg(&source.path)
        .args([
            "-map",
            "0:a:0",
            "-vn",
            "-c:a",
            "pcm_s16le",
            "-f",
            "s16le",
            "-ac",
            "2",
            "-ar",
            "48000",
            "pipe:1",
        ]);
    cmd
}

fn video_command(source: &SandboxedMedia, start_tick: u32, width: u32, height: u32) -> Command {
    let mut cmd = Command::new(media_tool("STRATA_FFMPEG", "ffmpeg.exe"));
    cmd.arg("-v")
        .arg("error")
        .arg("-ss")
        .arg(format!("{:.3}", f64::from(start_tick) / f64::from(FPS)))
        .arg("-i")
        .arg(&source.path)
        .args(["-map", "0:v:0", "-an"])
        .arg("-vf")
        // `fitted_video_size` has already chosen dimensions with the source
        // aspect ratio. Scale to those exact dimensions so rawvideo emits a
        // fixed-size frame without a black pad or a short final read.
        .arg(format!("scale={width}:{height},format=rgba"))
        .args(["-f", "rawvideo", "-r", "30", "pipe:1"]);
    cmd
}

struct ProbeCacheEntry {
    path: PathBuf,
    stamp: strata_native_io::windows_file_stamp::FileStamp,
    executable: PathBuf,
    arguments: Vec<String>,
    stdout: Vec<u8>,
}

static PROBE_CACHE: LazyLock<Mutex<Vec<ProbeCacheEntry>>> = LazyLock::new(Mutex::default);

fn run_ffprobe(arguments: &[&str], path: &Path) -> Option<strata_win_shell::SandboxedOutput> {
    let executable = PathBuf::from(media_tool("STRATA_FFPROBE", "ffprobe.exe"));
    let stamp = strata_native_io::windows_file_stamp::from_metadata(&std::fs::metadata(path).ok()?);
    if let Ok(cache) = PROBE_CACHE.lock()
        && let Some(entry) = cache.iter().find(|entry| {
            entry.path == path
                && entry.stamp == stamp
                && entry.executable == executable
                && entry.arguments == arguments
        })
    {
        return Some(strata_win_shell::SandboxedOutput {
            exit_code: 0,
            stdout: entry.stdout.clone(),
            stderr: Vec::new(),
        });
    }
    let mut args: Vec<OsString> = arguments.iter().map(OsString::from).collect();
    args.push(path.as_os_str().to_owned());
    let arg_refs: Vec<&std::ffi::OsStr> = args.iter().map(OsString::as_os_str).collect();
    let limits = strata_win_shell::JobLimits {
        memory_limit_bytes: Some(256 * 1024 * 1024),
        active_process_limit: Some(1),
        kill_on_job_close: true,
        restrict_ui: true,
    };
    let output = strata_win_shell::run_in_job(
        &executable,
        &arg_refs,
        None,
        &limits,
        std::time::Duration::from_secs(5),
        None,
    )
    .ok()?;
    if output.exit_code == 0
        && let Ok(mut cache) = PROBE_CACHE.lock()
    {
        cache.retain(|entry| {
            entry.path != path || (entry.stamp == stamp && entry.arguments != arguments)
        });
        if cache.len() >= 32 {
            cache.remove(0);
        }
        cache.push(ProbeCacheEntry {
            path: path.to_owned(),
            stamp,
            executable,
            arguments: arguments
                .iter()
                .map(|argument| (*argument).to_owned())
                .collect(),
            stdout: output.stdout.clone(),
        });
    }
    Some(output)
}

fn probe_video_dimensions(path: &std::path::Path) -> Result<(f64, f64), String> {
    let output = run_ffprobe(
        &[
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,sample_aspect_ratio:stream_side_data=rotation",
            "-of",
            "json",
        ],
        path,
    )
    .ok_or_else(|| crate::i18n::tr("Unable to inspect media dimensions"))?;
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())?;
    let stream = &value["streams"][0];
    let width = stream["width"].as_u64().unwrap_or(0);
    let height = stream["height"].as_u64().unwrap_or(0);
    if output.exit_code != 0
        || width == 0
        || height == 0
        || width
            .checked_mul(height)
            .is_none_or(|pixels| pixels > 50_000_000)
    {
        return Err(crate::i18n::tr("Unsupported source dimensions"));
    }
    let sar = stream["sample_aspect_ratio"]
        .as_str()
        .and_then(|sar| sar.split_once(':'))
        .and_then(|(n, d)| Some(n.parse::<f64>().ok()? / d.parse::<f64>().ok()?))
        .filter(|sar| sar.is_finite() && *sar > 0.0 && *sar < 100.0)
        .unwrap_or(1.0);
    let (mut width, mut height) = (width as f64 * sar, height as f64);
    if stream["side_data_list"].as_array().is_some_and(|data| {
        data.iter().any(|entry| {
            entry["rotation"]
                .as_i64()
                .is_some_and(|rotation| rotation.rem_euclid(180) == 90)
        })
    }) {
        std::mem::swap(&mut width, &mut height);
    }
    Ok((width, height))
}

fn fitted_video_size(
    requested_width: u32,
    requested_height: u32,
    source_width: f64,
    source_height: f64,
) -> (u32, u32) {
    let scale = (f64::from(requested_width) / source_width)
        .min(f64::from(requested_height) / source_height)
        .min(1.0);
    (
        (source_width * scale).floor().max(1.0) as u32,
        (source_height * scale).floor().max(1.0) as u32,
    )
}

fn has_audio(path: &std::path::Path) -> bool {
    run_ffprobe(
        &[
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=index",
            "-of",
            "csv=p=0",
        ],
        path,
    )
    .is_some_and(|result| result.exit_code == 0 && !result.stdout.is_empty())
}

fn decode_video(
    source: SandboxedMedia,
    start_tick: u32,
    cancelled: &AtomicBool,
    sender: &mpsc::SyncSender<Event>,
    children: &Arc<Mutex<Vec<Arc<strata_win_shell::SandboxedChild>>>>,
) -> Result<(), String> {
    if source.audio_only {
        return decode_audio(source, start_tick, cancelled, sender, children);
    }
    let requested_width = source.size.width.clamp(16, 1280) as u32;
    let requested_height = source.size.height.clamp(16, 1280) as u32;
    let (source_width, source_height) = probe_video_dimensions(&source.path)?;
    let (width, height) = fitted_video_size(
        requested_width,
        requested_height,
        source_width,
        source_height,
    );
    let duration_us = probe_duration(&source.path).unwrap_or(MAX_DURATION_US);
    let audio = has_audio(&source.path);
    let pixels_pipe = spawn_decoder(
        &mut video_command(&source, start_tick, width, height),
        cancelled,
        children,
    )?;
    // Use separate bounded raw PCM and raw RGBA streams. They are decoded only
    // once each, never PNG/JPEG-encoded and never multiplexed through the GTK UI.
    let mut audio_pipe = if audio {
        Some(spawn_decoder(
            &mut audio_command(&source, start_tick),
            cancelled,
            children,
        )?)
    } else {
        None
    };
    let header = Header {
        width,
        height,
        audio,
        duration_us,
        start_tick,
    };
    let frame_bytes = header.video_bytes();
    send(sender, Event::Prepared(header), cancelled)?;
    let mut tick = start_tick;
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        let pixels = read_chunk(pixels_pipe.as_ref(), frame_bytes)?;
        if pixels.is_empty() {
            break;
        }
        if pixels.len() != frame_bytes {
            return Err("Incomplete decoded video frame".into());
        }
        let samples = if let Some(pipe) = audio_pipe.as_mut() {
            let count = crate::media::audio_bytes(header, tick);
            let mut pcm = read_chunk(pipe.as_ref(), count)?;
            // Silence-pad a missing/short track instead of blocking the video
            // and keep the wire protocol's exact, contiguous PCM frame sizes.
            pcm.resize(count, 0);
            pcm
        } else {
            Vec::new()
        };
        send(
            sender,
            Event::Packet(Packet::Frame(Frame {
                tick,
                pixels,
                samples,
            })),
            cancelled,
        )?;
        tick = tick.saturating_add(1);
    }
    if tick == start_tick {
        return Err("Video decoder returned no frames".into());
    }
    send(
        sender,
        Event::Packet(Packet::End(if duration_us == MAX_DURATION_US {
            crate::media::timestamp(tick)
        } else {
            duration_us.min(crate::media::timestamp(tick))
        })),
        cancelled,
    )
}

fn decode_audio(
    source: SandboxedMedia,
    start_tick: u32,
    cancelled: &AtomicBool,
    sender: &mpsc::SyncSender<Event>,
    children: &Arc<Mutex<Vec<Arc<strata_win_shell::SandboxedChild>>>>,
) -> Result<(), String> {
    let duration_us = probe_duration(&source.path).unwrap_or(MAX_DURATION_US);
    let pipe = spawn_decoder(&mut audio_command(&source, start_tick), cancelled, children)?;
    let header = Header {
        width: 0,
        height: 0,
        audio: true,
        duration_us,
        start_tick,
    };
    send(sender, Event::Prepared(header), cancelled)?;
    let mut tick = start_tick;
    let mut total_pcm_bytes = 0u64;
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        let wanted = crate::media::audio_bytes(header, tick);
        let mut samples = read_chunk(pipe.as_ref(), wanted)?;
        if samples.is_empty() {
            break;
        }
        samples.truncate(samples.len() & !3);
        if samples.is_empty() {
            break;
        }
        total_pcm_bytes += samples.len() as u64;
        let at_end = samples.len() < wanted;
        if tick == start_tick {
            // The first packet must cover AUDIO_LEAD_TICKS even for a sub-2s
            // clip. The UI trims the padded PCM to the probed media duration.
            samples.resize(wanted, 0);
        }
        send(
            sender,
            Event::Packet(Packet::Frame(Frame {
                tick,
                pixels: Vec::new(),
                samples,
            })),
            cancelled,
        )?;
        tick = tick.saturating_add(1);
        if at_end {
            break;
        }
    }
    if tick == start_tick {
        return Err("Audio decoder returned no PCM samples".into());
    }
    let decoded_end = crate::media::timestamp(start_tick)
        + total_pcm_bytes * 1_000_000 / (crate::media::SAMPLE_RATE * 4);
    send(
        sender,
        Event::Packet(Packet::End(if duration_us == MAX_DURATION_US {
            decoded_end
        } else {
            duration_us.min(decoded_end)
        })),
        cancelled,
    )
}

fn media_tool(override_name: &str, executable: &str) -> std::ffi::OsString {
    if let Some(value) = std::env::var_os(override_name) {
        return value;
    }
    if let Some(tool) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(executable)))
        .filter(|tool| tool.is_file())
    {
        return tool.into_os_string();
    }
    executable.into()
}

fn probe_duration(path: &std::path::Path) -> Option<u64> {
    let output = run_ffprobe(
        &[
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ],
        path,
    )?;
    if output.exit_code != 0 {
        return None;
    }
    let seconds = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .ok()?;
    (seconds.is_finite() && seconds > 0.0)
        .then_some((seconds * 1_000_000.0).min(MAX_DURATION_US as f64) as u64)
}

pub(crate) enum PeaksEvent {
    Levels { start: u32, levels: Vec<u8> },
    Finished,
    Failed,
}

static ACTIVE_BACKGROUND: AtomicBool = AtomicBool::new(false);

pub(crate) struct PeaksSession {
    cancelled: Arc<AtomicBool>,
    receiver: mpsc::Receiver<PeaksEvent>,
}

impl PeaksSession {
    pub fn start(source: SandboxedMedia) -> Option<Self> {
        if ACTIVE_BACKGROUND
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return None;
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        let (sender, receiver) = mpsc::sync_channel(64);
        thread::Builder::new()
            .name("windows-audio-peaks".into())
            .spawn(move || {
                let result = render_peaks(&source, &stop, &sender);
                ACTIVE_BACKGROUND.store(false, Ordering::Release);
                let event = if result.is_ok() {
                    PeaksEvent::Finished
                } else {
                    PeaksEvent::Failed
                };
                let _ = sender.send(event);
            })
            .ok()?;
        Some(Self {
            cancelled,
            receiver,
        })
    }
    pub fn receive(&self) -> Option<PeaksEvent> {
        match self.receiver.try_recv() {
            Ok(event) => Some(event),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(PeaksEvent::Failed),
        }
    }
}

impl Drop for PeaksSession {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

pub(crate) enum StoryboardEvent {
    Sheet(crate::media::storyboard::Sheet),
    Cell { index: u32, pixels: Vec<u8> },
    Finished,
    Failed,
}

pub(crate) struct StoryboardSession {
    cancelled: Arc<AtomicBool>,
    receiver: mpsc::Receiver<StoryboardEvent>,
}

impl StoryboardSession {
    pub fn start(source: SandboxedMedia, cell_edge: u32) -> Option<Self> {
        if ACTIVE_BACKGROUND
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return None;
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        let (sender, receiver) = mpsc::sync_channel(64);
        thread::Builder::new()
            .name("windows-video-storyboard".into())
            .spawn(move || {
                let result = render_storyboard(&source, cell_edge, &stop, &sender);
                ACTIVE_BACKGROUND.store(false, Ordering::Release);
                let event = if result.is_ok() {
                    StoryboardEvent::Finished
                } else {
                    StoryboardEvent::Failed
                };
                let _ = sender.send(event);
            })
            .ok()?;
        Some(Self {
            cancelled,
            receiver,
        })
    }
    pub fn receive(&self) -> Option<StoryboardEvent> {
        match self.receiver.try_recv() {
            Ok(event) => Some(event),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(StoryboardEvent::Failed),
        }
    }
}

impl Drop for StoryboardSession {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

fn render_peaks(
    source: &SandboxedMedia,
    cancelled: &AtomicBool,
    sender: &mpsc::SyncSender<PeaksEvent>,
) -> Result<(), String> {
    let duration = probe_duration(&source.path).unwrap_or(MAX_DURATION_US - 1);
    let mut command = Command::new(media_tool("STRATA_FFMPEG", "ffmpeg.exe"));
    command
        .args(["-v", "error", "-i"])
        .arg(&source.path)
        .args(["-vn", "-ac", "1", "-ar", "8000", "-f", "s16le", "pipe:1"]);
    let children = Arc::new(Mutex::new(Vec::new()));
    let child = spawn_decoder(&mut command, cancelled, &children)?;
    let mut accumulator = crate::media::peaks::Accumulator::new(duration);
    let mut wire = Vec::new();
    let mut decoded_any = false;
    crate::media::peaks::write_header(&mut wire, duration).map_err(|e| e.to_string())?;
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err("Preview cancelled".into());
        }
        let pcm = read_chunk(child.as_ref(), 64 * 1024)?;
        if pcm.is_empty() {
            break;
        }
        decoded_any = true;
        accumulator
            .push(&pcm, &mut wire)
            .map_err(|error| error.to_string())?;
    }
    accumulator
        .finish(&mut wire)
        .map_err(|error| error.to_string())?;
    if !decoded_any {
        terminate_children(&children);
        return Err("Audio decoder returned no PCM samples".into());
    }
    terminate_children(&children);
    let mut cursor = std::io::Cursor::new(wire);
    crate::media::peaks::read_header(&mut cursor).map_err(|e| e.to_string())?;
    let mut runs = crate::media::peaks::RunReader::default();
    while let Some((start, levels)) = runs.read(&mut cursor).map_err(|e| e.to_string())? {
        sender
            .send(PeaksEvent::Levels { start, levels })
            .map_err(|_| "Waveform preview closed".to_owned())?;
    }
    Ok(())
}

fn render_storyboard(
    source: &SandboxedMedia,
    cell_edge: u32,
    cancelled: &AtomicBool,
    sender: &mpsc::SyncSender<StoryboardEvent>,
) -> Result<(), String> {
    let duration = probe_duration(&source.path).unwrap_or(MAX_DURATION_US - 1);
    let count = crate::media::storyboard::cell_count(duration);
    let edge = cell_edge.clamp(
        crate::media::storyboard::MIN_CELL_EDGE,
        crate::media::storyboard::MAX_CELL_EDGE,
    );
    let (source_width, source_height) =
        probe_video_dimensions(&source.path).unwrap_or((f64::from(edge), f64::from(edge)));
    let (width, height) = fitted_video_size(edge, edge, source_width, source_height);
    let sheet = crate::media::storyboard::Sheet {
        width,
        height,
        count,
        duration_us: duration,
    }
    .validate()
    .map_err(|error| error.to_string())?;
    sender
        .send(StoryboardEvent::Sheet(sheet))
        .map_err(|_| "Storyboard preview closed".to_owned())?;
    let order = crate::media::storyboard::subdivision_order(count);
    for index in order {
        if cancelled.load(Ordering::Acquire) {
            return Err("Preview cancelled".into());
        }
        let timestamp = sheet.cell_time_us(index);
        let mut command = Command::new(media_tool("STRATA_FFMPEG", "ffmpeg.exe"));
        command
            .args(["-v", "error", "-ss"])
            .arg(format!("{:.3}", timestamp as f64 / 1_000_000.0))
            .args(["-i"])
            .arg(&source.path)
            .args(["-frames:v", "1", "-vf"])
            .arg(format!(
                "scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black,format=rgba"
            ))
            .args(["-f", "rawvideo", "pipe:1"]);
        let children = Arc::new(Mutex::new(Vec::new()));
        let child = spawn_decoder(&mut command, cancelled, &children)?;
        let pixels = read_chunk(child.as_ref(), sheet.cell_bytes())?;
        terminate_children(&children);
        if pixels.len() == sheet.cell_bytes() {
            sender
                .send(StoryboardEvent::Cell { index, pixels })
                .map_err(|_| "Storyboard preview closed".to_owned())?;
        }
    }
    Ok(())
}
